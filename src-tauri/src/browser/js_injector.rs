// JsInjector: drives whatsapp web via CDP evaluate.
// bootstrap: login detection via localStorage (no deps), then WPP.js injection
// (from local disk cache or CDN) before any WPP/WAPI call.
use anyhow::{Context, Result};
use chromiumoxide::Page;
use serde::Deserialize;
use serde_json::Value;

use crate::message::variables::js_escape;

const WPP_CDN: &str = "https://unpkg.com/@wppconnect/wa-js/dist/wppconnect-wa.js";

pub struct JsInjector {
    pub page: Page,
    wpp_injected: bool,
}

#[derive(Debug, Deserialize)]
pub struct SendResult {
    #[serde(default)]
    pub sent_status: bool,
    #[serde(default, rename = "sentStatus")]
    pub sent_status_camel: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
}

impl SendResult {
    pub fn ok(&self) -> bool {
        self.sent_status || self.sent_status_camel.unwrap_or(false)
    }
}

impl JsInjector {
    pub fn new(page: &Page) -> Self {
        Self {
            page: page.clone(),
            wpp_injected: false,
        }
    }

    async fn eval_json(&self, script: &str) -> Result<Value> {
        let result = self
            .page
            .evaluate(script)
            .await
            .context("cdp evaluate failed")?;
        let v = result.into_value::<Value>().context("parsing cdp value")?;
        Ok(v)
    }

    // ---------- login detection (no library needed) ----------

    /// login detection: modern wa web keeps session in IndexedDB (not
    /// localStorage), so we check for the chat list DOM instead.
    pub async fn is_logged_in(&self) -> Result<bool> {
        let v = self
            .eval_json(
                r#"(function(){
                    try {
                        var wpp = window.WPP;
                        if (wpp && wpp.conn) {
                            if (typeof wpp.conn.isAuthenticated === 'function' && wpp.conn.isAuthenticated()) return true;
                            if (typeof wpp.conn.isMainReady === 'function' && wpp.conn.isMainReady()) return true;
                        }
                        return !!document.querySelector(
                            '#pane-side, #side, [data-testid="chat-list"], [aria-label="Chat list"], [aria-label="Daftar chat"]'
                        );
                    } catch(e) { return false; }
                })()"#,
            )
            .await?;
        Ok(v.as_bool().unwrap_or(false))
    }

    pub async fn my_user_id(&self) -> Result<String> {
        // prefer the WPP identity (reliable once wa-js is injected);
        // fall back to the legacy localStorage key on older wa web builds
        let v = self
            .eval_json(
                r#"(function(){
                    try {
                        if (window.WPP && window.WPP.isReady) {
                            var u = WPP.conn.getMyUserId();
                            if (u && u.user) return String(u.user);
                        }
                    } catch(e) {}
                    try {
                        var w = window.localStorage.getItem('last-wid') || '';
                        return w.split('@')[0].split(':')[0];
                    } catch(e) { return ''; }
                })()"#,
            )
            .await?;
        Ok(v.as_str().unwrap_or("").to_string())
    }

    // ---------- WPP bootstrap ----------

    /// inject WPP.js (wa-js) into the page and wait until ready.
    /// web.whatsapp.com has strict CSP that blocks <script src> injection,
    /// so we fetch the bundle in Rust and execute it via Runtime.evaluate
    /// (same mechanism as devtools console — CSP does not apply).
    pub async fn ensure_wpp(&mut self, local_wpp_js: Option<&str>) -> Result<()> {
        if self.wpp_injected {
            return Ok(());
        }

        // already loaded from a previous injection?
        let already = self
            .eval_json(
                r#"(function(){ try { return !!(window.WPP && window.WPP.isReady); } catch(e){ return false; } })()"#,
            )
            .await?
            .as_bool()
            .unwrap_or(false);
        if already {
            let _ = self
                .eval_json(
                    r#"(async function(){ try { await window.WPP.init(); } catch(e){} return true; })()"#,
                )
                .await;
            self.wpp_injected = true;
            log::info!("WPP.js already present and ready");
            return Ok(());
        }

        // source: disk cache first, then CDN fetch in rust (bypasses page CSP)
        let source: String = match local_wpp_js {
            Some(code) if code.len() > 1000 => code.to_string(),
            _ => {
                log::info!("fetching WPP.js from CDN ({WPP_CDN})");
                let resp = reqwest::Client::new()
                    .get(WPP_CDN)
                    .header("User-Agent", "BlastWA/0.1")
                    .send()
                    .await
                    .context("downloading WPP.js")?
                    .error_for_status()
                    .context("WPP.js download http error")?;
                resp.text().await.context("reading WPP.js body")?
            }
        };
        log::info!("executing WPP.js bundle ({} kb) via CDP evaluate", source.len() / 1024);

        // execute the bundle in page main world via CDP — but transfer it in
        // chunks: a single multi-megabyte Runtime.evaluate resets the
        // chromiumoxide websocket mid-flight, killing every later call with
        // "cdp evaluate failed". assemble the source in a page variable and
        // indirect-eval it once complete.
        const CHUNK: usize = 256 * 1024;
        let wrapped = format!("(function(){{\n{}\n}})()", source);
        self.eval_json("window.__bw_wpp = ''; window.__bw_wpp.length").await?;
        let mut start = 0;
        while start < wrapped.len() {
            let mut end = (start + CHUNK).min(wrapped.len());
            while end < wrapped.len() && !wrapped.is_char_boundary(end) {
                end += 1;
            }
            let lit = serde_json::to_string(&wrapped[start..end])?;
            self.eval_json(&format!("window.__bw_wpp += {}; window.__bw_wpp.length", lit))
                .await?;
            start = end;
        }
        self.eval_json(
            "(function(){ try { (0,eval)(window.__bw_wpp); return true; } finally { try { delete window.__bw_wpp; } catch(e) {} } })()",
        )
        .await
        .context("executing WPP.js bundle via cdp")?;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(90);
        let mut check_interval = std::time::Duration::from_millis(300);
        loop {
            let ready = self
                .eval_json(
                    r#"(function(){ 
                        try { 
                            if (window.WPP && window.WPP.isReady) return true;
                            if (window.WPP && typeof window.WPP.init === 'function') {
                                window.WPP.init().catch(function(){});
                            }
                            return false;
                        } catch(e){ return false; } 
                    })()"#,
                )
                .await?
                .as_bool()
                .unwrap_or(false);
            if ready {
                self.wpp_injected = true;
                log::info!("WPP.js injected and ready (adaptive bootstrap)");
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("WPP.js did not become ready (adaptive timeout reached)");
            }
            tokio::time::sleep(check_interval).await;
            if check_interval < std::time::Duration::from_millis(1000) {
                check_interval += std::time::Duration::from_millis(100);
            }
        }
    }

    // ---------- messaging (WPP first, WAPI fallback) ----------

    pub async fn send_message(
        &self,
        wa_id: &str,
        message: &str,
        is_safe: bool,
    ) -> Result<SendResult> {
        let id_json = serde_json::to_string(wa_id)?;
        let msg_json = serde_json::to_string(message)?;
        let script = format!(
            r#"(async () => {{
                var id = {id_json};
                var msg = {msg_json};
                try {{
                    if ({is_safe}) {{
                        var exists = await WPP.contact.queryExists(id);
                        if (!exists) return {{ sentStatus: false, error: "chat not found" }};
                    }}
                    var r = await WPP.chat.sendTextMessage(id, msg);
                    return {{ sentStatus: true, result: r || null }};
                }} catch (e1) {{
                    try {{
                        var r2 = await WAPI.sendMessage(id, msg);
                        return {{ sentStatus: !(r2 && r2.erro) }};
                    }} catch (e2) {{
                        return {{ sentStatus: false, error: String(e1) }};
                    }}
                }}
            }})()"#,
            id_json = id_json,
            msg_json = msg_json,
            is_safe = is_safe,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn send_file(
        &self,
        wa_id: &str,
        data_uri: &str,
        filename: &str,
        caption: &str,
        is_safe: bool,
    ) -> Result<SendResult> {
        let id = js_escape(wa_id);
        let b64 = js_escape(data_uri);
        let fname = js_escape(filename);
        let cap = js_escape(caption);
        let script = format!(
            r#"(async () => {{
                function getFileOpts(n) {{
                    var e = n.split('.').pop().toLowerCase();
                    var m = {{ jpg:'image',jpeg:'image',png:'image',webp:'image',
                              mp4:'video',mov:'video',avi:'video',mp3:'audio',
                              pdf:'document',doc:'document',docx:'document',xls:'document',
                              xlsx:'document',ppt:'document',pptx:'document',txt:'document' }};
                    var o = {{ type: m[e] || 'document', caption: '{cap}', filename: '{fn}' }};
                    // wa-js v4.6: animated gifs ride the video path with isGif
                    // (bundle media prep sets asGif from options.isGif)
                    if (e === 'gif') {{ o.type = 'video'; o.isGif = true; }}
                    return o;
                }}
                try {{
                    if ({is_safe}) {{
                        var exists = await WPP.contact.queryExists('{id}');
                        if (!exists) return {{ sentStatus: false, error: "chat not found" }};
                    }}
                    await WPP.chat.sendFileMessage('{id}', '{b64}', getFileOpts('{fn}'));
                    return {{ sentStatus: true }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id,
            b64 = b64,
            cap = cap,
            fn = fname,
            is_safe = is_safe,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn send_ptt(&self, wa_id: &str, data_uri: &str) -> Result<SendResult> {
        let id = js_escape(wa_id);
        let b64 = js_escape(data_uri);
        let script = format!(
            r#"(async () => {{
                try {{
                    await WPP.chat.sendFileMessage('{id}', '{b64}', {{ type: 'audio', isPtt: true }});
                    return {{ sentStatus: true }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id,
            b64 = b64,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    // ---------- number checking ----------

    pub async fn check_number(&self, number: &str) -> Result<NumberStatus> {
        let clean = number.replace('+', "");
        let script = format!(
            r#"(async () => {{
                try {{
                    // wa-js v4.6: queryExists returns an object (wid, biz, lid...)
                    // on hit, null when the number has no whatsapp account. there
                    // is no `.exists` field anymore (older bundles had one).
                    var r = await WPP.contact.queryExists('{n}@c.us');
                    return {{
                        numtoCheck: '{n}',
                        exists: !!r,
                        canReceiveMessage: !!r,
                        isBusiness: !!(r && r.business),
                        wid: r ? String(r.wid || '') : ''
                    }};
                }} catch (ex) {{
                    return {{ numtoCheck: '{n}', error: String(ex) }};
                }}
            }})()"#,
            n = clean
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    // ---------- groups ----------

    /// U14: contacts saved in the account's whatsapp phonebook
    pub async fn list_wa_contacts(&self) -> Result<Vec<(String, String)>> {
        let v = self.eval_json(
            r#"(async () => {
                try {
                    var cs = await WPP.contact.list({ onlyMyContacts: true });
                    return (cs || []).map(function (c) {
                        return {
                            number: String((c.id && c.id.user) || ''),
                            name: String(c.name || c.formattedName || c.pushname || '')
                        };
                    });
                } catch (ex) { return { error: String(ex) }; }
            })()"#,
        ).await?;
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            anyhow::bail!("list contacts failed: {err}");
        }
        let items = v.as_array().cloned().unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|c| {
                let number = c.get("number")?.as_str()?.to_string();
                let name = c
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                if number.is_empty() {
                    None
                } else {
                    Some((number, name))
                }
            })
            .collect())
    }

    /// U15: interactive list message (buttons are gone from wa-js v4+;
    /// list messages are the supported interactive primitive)
    pub async fn send_list_message(
        &self,
        wa_id: &str,
        title: &str,
        description: &str,
        button_text: &str,
        footer: &str,
        sections_json: &str,
    ) -> Result<SendResult> {
        let id = js_escape(wa_id);
        let title = js_escape(title);
        let desc = js_escape(description);
        let btn = js_escape(button_text);
        let footer = js_escape(footer);
        let script = format!(
            r#"(async () => {{
                try {{
                    await WPP.chat.sendListMessage('{id}', {{
                        title: '{title}',
                        description: '{desc}',
                        buttonText: '{btn}',
                        footer: '{footer}',
                        sections: {sections}
                    }});
                    return {{ sentStatus: true }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id,
            title = title,
            desc = desc,
            btn = btn,
            footer = footer,
            sections = sections_json,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    /// U16: products in this account's own catalog
    pub async fn get_catalog_products(&self) -> Result<Vec<(String, String, String)>> {
        let v = self.eval_json(
            r#"(async () => {
                try {
                    var raw = await WPP.catalog.getMyCatalog();
                    var arr = Array.isArray(raw) ? raw : (raw && raw.data) ||
                              (raw && raw.msgProductCollection && raw.msgProductCollection._models) || [];
                    return (arr || []).map(function (p) {
                        return {
                            id: String(p.id || ''),
                            name: String(p.name || ''),
                            description: String(p.description || '')
                        };
                    });
                } catch (ex) { return { error: String(ex) }; }
            })()"#,
        ).await?;
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            anyhow::bail!("catalog fetch failed: {err}");
        }
        let items = v.as_array().cloned().unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|p| {
                let id = p.get("id")?.as_str()?.to_string();
                let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let description = p
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((id, name, description))
            })
            .collect())
    }

    /// U16: catalog product card message
    pub async fn send_catalog_message(&self, wa_id: &str, product_id: &str) -> Result<SendResult> {
        let id = js_escape(wa_id);
        let pid = js_escape(product_id);
        let script = format!(
            r#"(async () => {{
                try {{
                    await WPP.chat.sendCatalogMessage('{id}', '{pid}', {{}});
                    return {{ sentStatus: true }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id,
            pid = pid,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    // ---------- groups ----------

    pub async fn get_all_groups(&self) -> Result<Vec<(String, String)>> {
        let v = self
            .eval_json(
                r#"(async function(){
                    try {
                        var gs = await WPP.group.getAllGroups();
                        return gs.map(function(g){
                            var id = (g.id && (g.id._serialized || g.id.id)) || g.gid || '';
                            var name = g.formattedTitle || (g.contact && g.contact.name) || g.name || g.subject || '';
                            return {id: String(id), name: String(name)};
                        });
                    } catch(e) { return []; }
                })()"#,
            )
            .await?;
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for g in arr {
                out.push((
                    g["id"].as_str().unwrap_or("").to_string(),
                    g["name"].as_str().unwrap_or("").to_string(),
                ));
            }
        }
        Ok(out)
    }

    pub async fn get_group_participants(&self, group_id: &str) -> Result<Vec<(String, Option<String>)>> {
        let gid = js_escape(group_id);
        let script = format!(
            r#"(async () => {{
                try {{
                    var p = await WPP.group.getParticipants('{gid}');
                    // whatsapp's lid era: participant ids often come back as
                    // @lid linked identities with no phone number attached.
                    // resolve each through the contact store; unresolved lids
                    // are skipped since a blast to a lid id cannot be routed.
                    var rows = await Promise.all(p.map(async function(x) {{
                        var sid = (x.id && x.id._serialized) ? String(x.id._serialized) : String(x);
                        var push = x.pushname || x.name || null;
                        if (sid.indexOf('@lid') === -1) {{
                            return {{ number: sid, name: push }};
                        }}
                        try {{
                            var c = await WPP.contact.get(sid);
                            if (c && c.phoneNumber && c.phoneNumber._serialized) {{
                                var cname = c.name || c.pushName || push || null;
                                return {{ number: String(c.phoneNumber._serialized), name: cname }};
                            }}
                        }} catch (e) {{}}
                        return null;
                    }}));
                    return rows.filter(function(x) {{ return x !== null; }});
                }} catch (ex) {{ return []; }}
            }})()"#,
            gid = gid
        );
        let v = self.eval_json(&script).await?;
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for o in arr {
                let number = o.get("number").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if number.is_empty() {
                    continue;
                }
                let name = o
                    .get("name")
                    .and_then(|x| x.as_str())
                    .map(|x| x.to_string())
                    .filter(|x| !x.is_empty());
                out.push((number, name));
            }
        }
        Ok(out)
    }

    // ---------- presence (best effort) ----------

    pub async fn mark_seen(&self, wa_id: &str) -> Result<()> {
        let _ = self
            .eval_json(&format!(
                r#"(async function(){{ try {{ await WPP.chat.markIsRead('{id}@c.us'); }} catch(e){{}} return true; }})()"#,
                id = js_escape(wa_id)
            ))
            .await;
        Ok(())
    }

    pub async fn send_typing_state(&self, wa_id: &str) -> Result<()> {
        let _ = self
            .eval_json(&format!(
                r#"(async function(){{ try {{ await WPP.chat.markIsComposing('{id}@c.us'); }} catch(e){{}} return true; }})()"#,
                id = js_escape(wa_id)
            ))
            .await;
        Ok(())
    }

    pub async fn poll_new_messages(&self) -> Result<Value> {
        self.eval_json(
            r#"(function(){ try { return WAPI.getAllChatsWithNewMsg(null) || []; } catch(e){ return []; } })()"#,
        )
        .await
    }
}

// NumberStatus lives at module level (see below) — keep the impl near RawCheck
#[derive(Debug, Deserialize)]
pub struct NumberStatus {
    #[serde(rename = "numtoCheck", default)]
    pub number: String,
    #[serde(default)]
    pub exists: Option<bool>,
    #[serde(rename = "canReceiveMessage", default)]
    pub can_receive_message: Option<bool>,
    #[serde(rename = "isBusiness", default)]
    pub is_business: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
}

impl NumberStatus {
    pub fn exists(&self) -> bool {
        self.exists.unwrap_or(false) || self.can_receive_message.unwrap_or(false)
    }

    pub fn kind(&self) -> &'static str {
        if self.is_business.unwrap_or(false) {
            "Business"
        } else if self.exists() {
            "Regular"
        } else {
            "Not Found"
        }
    }
}
