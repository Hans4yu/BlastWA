---
title: "research: OKESENDER familiar accounts (FrmAdvanced) RE findings"
type: research
status: done
date: 2026-08-25
---

# U19 Research: Familiar Accounts

## Method

UTF-16LE string extraction from `D:/Tes/OKESENDER.exe` (1.3MB), filtered on
Familiar/Advanced/Blind/Button/Catalog terms via `extract_okesender_strings.js`.

## Findings

### What familiar accounts actually are

NOT license-server noise. FrmAdvanced ("Sending Settings") implements a
humanization/engagement feature:

1. `BtnAddFamiliarAccount` / `FrmAddFamiliarAccount`: the operator registers
   OTHER real WhatsApp accounts as "familiars", with a consent notice baked
   into the add dialog: "Please ensure that the account that you add it is a
   whatsapp account and he is aware that he is going to receive messages
   from you".
2. `LstMessages` + "Messages Dictionary" (`BtnAddMessages`): a pool of
   realistic messages attached per familiar number.
3. Timing controls: send to familiars "After: N Seconds", sleep between
   sending (CheckBox2), delay range "Wait between X and Y" (WaitFrom/WaitTo),
   connection speed preset Very Slow..Very Fast (`CmbSpeed`), optional
   "internal dialog" simulation (CheckBox1) with Count + Seconds between
   messages.

Semantics: between blasts, the sender account chats naturally with known,
consenting contacts so its behavioral profile looks like an ordinary user.
It is engagement warming, gated by explicit consent of both parties.

### Bonus evidence extracted alongside

- Three sending modes confirmed: RadioButtonSafeMode ("Sending Bulk - Safe
  Mode"), RadioButtonBlindMode ("Blind Mode"), Multi-Channel Mode
  ("will send you bulk from multiple accounts"; multi-channel runs in blind
  mode only).
- Blind mode exact copy: sends to ALL imported contacts "whatever is it
  valid or invalid, blind mode will not show the status of the message";
  safe contacts are defined as contacts you already had a conversation with.
- Buttons UI stores definitions in a "Buttons File" (Create/Edit Buttons);
  catalog in a "Catalog File" (Create/Edit Catalog).

## Go/No-Go decision

**GO, deferred implementation.** Semantics are clear and implementable with
existing primitives: a background loop picking dictionary messages per
familiar number through the normal send path with human-mode delays. Consent
notice must be preserved verbatim in the UI. Not scheduled for the current
parity phase; recorded as backlog candidate "Engager".

## BlastWA mapping notes

- BlastWA's blind mode (U12) skips existence pre-checks exactly like
  OKESENDER's, but keeps honest sent/failed counters instead of hiding them;
  deviation accepted deliberately (hiding failures misleads the operator).
- Multi-channel forcing blind mode (U17 design constraint) adopted.
