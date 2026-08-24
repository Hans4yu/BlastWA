// JsInjector: drives whatsapp web via CDP evaluate.
// templates are verbatim from the original binary's string heap, adapted to
// return values through CDP instead of window.chrome.webview.postMessage.
use anyhow::{Context, Result};
use chromiumoxide::Page;
use serde::Deserialize;
use serde_json::Value;

use crate::message::variables::js_escape;

pub struct JsInjector<'a> {
    pub page: &'a Page,
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

#[derive(Debug, Deserialize)]
pub struct NumberStatus {
    #[serde(rename = "numtoCheck", default)]
    pub number: String,
    #[serde(default)]
    pub can_receive_message: Option<bool>,
    #[serde(default, rename = "isBusiness")]
    pub is_business: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl NumberStatus {
    pub fn exists(&self) -> bool {
        self.can_receive_message.unwrap_or(false)
            || matches!(self.status.as_deref(), Some("Business") | Some("Regular"))
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

impl<'a> JsInjector<'a> {
    pub fn new(page: &'a Page) -> Self {
        Self { page }
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

    pub async fn is_logged_in(&self) -> Result<bool> {
        let v = self
            .eval_json(
                r#"(function(){ try { var m = WPP.conn.getMyUserId(); return m ? m.user : ""; } catch(e) { return ""; } })()"#,
            )
            .await?;
        Ok(v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
    }

    pub async fn my_user_id(&self) -> Result<String> {
        let v = self.eval_json("WPP.conn.getMyUserId().user").await?;
        Ok(v.as_str().unwrap_or("").to_string())
    }

    /// text message; is_safe verifies the chat exists before sending.
    pub async fn send_message(
        &self,
        wa_id: &str,
        message: &str,
        is_safe: bool,
    ) -> Result<SendResult> {
        let id_esc = js_escape(wa_id);
        let msg_esc = js_escape(message);
        // promise-aware wrapper: await inside an async IIFE so CDP resolves it
        let script = format!(
            r#"(async () => {{
                try {{
                    if ({is_safe}) {{
                        var chatId = null;
                        try {{ chatId = await WAPI.getchatId('{id}'); }} catch(e) {{}}
                        if (!chatId) return {{ sentStatus: false, error: "chat not found" }};
                    }}
                    var r = await WAPI.sendMessage('{id}', '{msg}');
                    return {{ sentStatus: !(r && r.erro), result: r || null }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id_esc,
            msg = msg_esc,
            is_safe = is_safe,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    /// file (image/video/document) via WPP with base64 data uri
    pub async fn send_file(
        &self,
        wa_id: &str,
        data_uri: &str,
        filename: &str,
        caption: &str,
        is_safe: bool,
    ) -> Result<SendResult> {
        let id_esc = js_escape(wa_id);
        let fn_esc = js_escape(filename);
        let cap_esc = js_escape(caption);
        // data uri goes in as a JS string literal too — escape it
        let b64_esc = js_escape(data_uri);
        let script = format!(
            r#"(async () => {{
                function getFileType(n) {{
                    var e = n.split('.').pop().toLowerCase();
                    var m = {{ jpg:'image',jpeg:'image',png:'image',gif:'image',webp:'image',
                              mp4:'video',mov:'video',avi:'video',mp3:'audio',ogg:'audio',
                              pdf:'document',doc:'document',docx:'document',xls:'document',
                              xlsx:'document',ppt:'document',pptx:'document',txt:'document' }};
                    return m[e] || 'document';
                }}
                try {{
                    if ({is_safe}) {{
                        var chatId = null;
                        try {{ chatId = await WAPI.getchatId('{id}'); }} catch(e) {{}}
                        if (!chatId) return {{ sentStatus: false, error: "chat not found" }};
                    }}
                    await WPP.chat.sendFileMessage('{id}', '{b64}', {{
                        type: getFileType('{fn}'),
                        caption: '{cap}',
                        filename: '{fn}'
                    }});
                    return {{ sentStatus: true }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id_esc,
            b64 = b64_esc,
            cap = cap_esc,
            fn = fn_esc,
            is_safe = is_safe,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    /// push-to-talk voice note
    pub async fn send_ptt(&self, wa_id: &str, data_uri: &str) -> Result<SendResult> {
        let id_esc = js_escape(wa_id);
        let b64_esc = js_escape(data_uri);
        let script = format!(
            r#"(async () => {{
                try {{
                    var chatId = await WAPI.getchatId('{id}');
                    if (!chatId) return {{ sentStatus: false, error: "chat not found" }};
                    await WPP.chat.sendFileMessage(chatId, '{b64}', {{ type: 'ptt', isPtt: true }});
                    return {{ sentStatus: true }};
                }} catch (ex) {{
                    return {{ sentStatus: false, error: String(ex) }};
                }}
            }})()"#,
            id = id_esc,
            b64 = b64_esc,
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn check_number(&self, number: &str) -> Result<NumberStatus> {
        let clean = number.replace('+', "");
        let script = format!(
            r#"(async () => {{
                try {{
                    var e = await WAPI.checkNumberStatus('{n}@c.us');
                    e.numtoCheck = '{n}';
                    return e;
                }} catch (ex) {{
                    return {{ numtoCheck: '{n}', error: String(ex) }};
                }}
            }})()"#,
            n = clean
        );
        let v = self.eval_json(&script).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn get_all_groups(&self) -> Result<Vec<(String, String)>> {
        let v = self
            .eval_json(
                r#"(function(){ var t=[]; try { for(let c of WAPI.getAllGroups()){ t.push({id:c.id._serialized,name:c.name}); } } catch(e){} return t; })()"#,
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

    pub async fn get_group_participants(&self, group_id: &str) -> Result<Vec<String>> {
        let gid = js_escape(group_id);
        let script = format!(
            r#"(async () => {{
                try {{
                    var p = await WPP.group.getParticipants('{gid}');
                    return p.map(x => x.id ? x.id._serialized : String(x));
                }} catch (ex) {{ return []; }}
            }})()"#,
            gid = gid
        );
        let v = self.eval_json(&script).await?;
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for s in arr {
                if let Some(str_s) = s.as_str() {
                    out.push(str_s.to_string());
                }
            }
        }
        Ok(out)
    }

    /// presence helpers used by the human behavior engine (best-effort)
    pub async fn mark_seen(&self, wa_id: &str) -> Result<()> {
        let _ = self
            .eval_json(&format!("try {{ WAPI.markSeen('{}'); }} catch(e) {{}} true", wa_id))
            .await;
        Ok(())
    }

    pub async fn send_typing_state(&self, wa_id: &str) -> Result<()> {
        let _ = self
            .eval_json(&format!(
                "try {{ WAPI.sendChatStateComposing('{}@c.us'); }} catch(e) {{}} true",
                wa_id
            ))
            .await;
        Ok(())
    }

    pub async fn poll_new_messages(&self) -> Result<Value> {
        // snapshot of chats with unread messages for autoreply watcher
        self.eval_json(
            r#"(function(){ try { return WAPI.getAllChatsWithNewMsg(null) || []; } catch(e){ return []; } })()"#,
        )
        .await
    }
}
