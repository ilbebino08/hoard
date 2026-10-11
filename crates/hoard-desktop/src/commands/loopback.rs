//! Loopback HTTP listener for the OAuth desktop handoff.
//!
//! Snap/Flatpak-confined browsers (Ubuntu ships Firefox as a snap by default)
//! cannot dispatch a custom `hoard://` URL scheme to the host, so the
//! browser-to-app handoff after a Supabase OAuth round-trip silently fails: the
//! web "success" page sets `window.location.href = "hoard://…"` and nothing
//! reaches the app. The standard desktop-OAuth workaround (RFC 8252) is a
//! loopback redirect: the app listens on `http://127.0.0.1:<ephemeral>` (a URL
//! confined browsers *can* open), the web callback redirects the freshly-minted
//! tokens there, and we feed them into the very same `deep-link://new-url` path
//! the rest of the app already consumes. The custom scheme stays as a fallback.

use std::time::Duration;

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// How long the listener waits for the browser to come back before giving up
/// and freeing the port. Generous, since the user may have to pick an account or
/// approve a provider consent screen first, and the email sign-in waits on a
/// message that can take minutes to arrive: at five minutes a late link landed
/// on a closed port, `ERR_CONNECTION_REFUSED` and nothing to explain it. An idle
/// loopback socket costs nothing. It also bounds how long `cloud_login_url`
/// keeps reusing the same in-flight attempt (nonce + port): past this window the
/// listener is gone, so a fresh attempt is minted.
pub(crate) const LISTEN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What the browser shows once the app has the tokens, in the browser's language.
/// The sentences are the web's own `callback.desktop_return`, so the two pages
/// the user sees one after the other say the same thing. This used to be a single
/// Spanish page, without its accents, for everybody.
fn return_sentence(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "en" => "Signed in! Head back to the Hoard app and close this tab.",
        "es" => "¡Sesión iniciada! Vuelve a la app de Hoard, ya puedes cerrar esta pestaña.",
        "de" => "Angemeldet! Geh zurück zur Hoard-App, diesen Tab kannst du schließen.",
        "fr" => "Connecté ! Retourne dans l'app Hoard et ferme cet onglet.",
        "it" => "Accesso effettuato! Torna all'app Hoard e chiudi questa scheda.",
        "ja" => "ログインしました！Hoardアプリに戻ってください。このタブは閉じて構いません。",
        "pt" => "Sessão iniciada! Volta à app Hoard e fecha este separador.",
        "zh" => "已登录！请回到 Hoard 应用，这个标签页可以关闭了。",
        _ => return None,
    })
}

const WAIT_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Hoard</title></head><body style=\"font-family:system-ui,sans-serif;background:#0a0a0a;\
color:#a1a1aa\"><p style=\"padding:2rem\">Waiting for the Hoard sign-in...</p>\
</body></html>";

/// The first language in `Accept-Language` that we have a sentence for, English
/// otherwise. Browsers list the tags in order of preference, so the q-values are
/// not read.
fn return_message(accept_language: Option<&str>) -> (String, &'static str) {
    accept_language
        .into_iter()
        .flat_map(|header| header.split(','))
        .filter_map(|tag| tag.split(';').next())
        .filter_map(|tag| tag.trim().split('-').next())
        .map(str::to_ascii_lowercase)
        .find_map(|lang| return_sentence(&lang).map(|sentence| (lang, sentence)))
        .unwrap_or_else(|| ("en".to_string(), return_sentence("en").unwrap_or_default()))
}

fn ok_page(accept_language: Option<&str>) -> String {
    let (lang, message) = return_message(accept_language);
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
<title>Hoard</title><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<style>body{{font-family:system-ui,-apple-system,sans-serif;background:#0a0a0a;color:#e5e5e5;\
display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}}\
p{{text-align:center;padding:2rem;font-size:1.25rem;max-width:32rem}}</style></head>\
<body><p>{message}</p></body></html>"
    )
}

/// Bind an ephemeral loopback port and spawn a one-shot HTTP server that
/// captures the OAuth callback. Takes the single-use `state` nonce minted by
/// `cloud_login_url` (the same nonce is also stored in `AppState` so the final
/// `cloud_complete_login` re-checks it). Returns the bound `port` the web side
/// must redirect to. The spawned task lives until it captures a matching
/// callback or the timeout elapses.
///
/// The `state` nonce is the CSRF guard (RFC 6749 §10.12, RFC 8252 §8.9):
/// without it, any local process (or a web page that guessed the ephemeral
/// port) could POST attacker-controlled tokens to `/callback` and silently
/// log the app into the attacker's account, sending the user's backups to it.
pub async fn start(app: AppHandle, state: String) -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("binding loopback listener")?;
    let port = listener.local_addr()?.port();
    tracing::info!(port, "loopback OAuth listener up");

    let expected_state = state;
    tauri::async_runtime::spawn(async move {
        let deadline = tokio::time::sleep(LISTEN_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => {
                    tracing::info!("loopback OAuth listener timed out");
                    break;
                }
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    match serve(stream, &expected_state).await {
                        Ok(Some((access, refresh))) => {
                            // The one line that says the browser leg worked: a
                            // sign-in that never gets here failed on the web.
                            tracing::info!("loopback OAuth: callback received, handing it to the app");
                            // Reuse the existing deep-link path: synthesize the
                            // same `hoard://auth/callback?…` URL the frontend's
                            // listener already parses, then bring the window up.
                            // Carry the already-validated `state` so the final
                            // `cloud_complete_login` re-checks it against the
                            // pending nonce, exactly like the direct hoard://
                            // fallback path.
                            let url = format!(
                                "hoard://auth/callback?access_token={access}&refresh_token={refresh}&state={expected_state}"
                            );
                            crate::commands::window::reveal_main(&app);
                            crate::capture_deep_link(&app, url, true);
                            // Deliberately NOT `break`. `cloud_login_url` keeps
                            // handing this same port out for the whole
                            // LISTEN_TIMEOUT window while the attempt is
                            // pending, so a one-shot listener left every retry
                            // inside that window aimed at a closed socket, so
                            // the browser got ERR_CONNECTION_REFUSED and the
                            // user had to wait out the 5 minutes. Capturing a
                            // callback is not the same as *completing* a login:
                            // an interrupted handoff, a failed `/v1/me`, or the
                            // user going back to pick a different account all
                            // land here with the attempt still live. Keep
                            // serving until it's retired or the deadline hits.
                            if !attempt_pending(&app, &expected_state) {
                                break;
                            }
                        }
                        // A stray request (a favicon probe, say): keep waiting.
                        Ok(None) => continue,
                        Err(e) => {
                            tracing::warn!(error = %e, "loopback OAuth request failed");
                            continue;
                        }
                    }
                }
            }
        }
    });

    Ok(port)
}

/// Is this listener's attempt still the one `cloud_login_url` advertises?
/// `cloud_complete_login` clears `pending_login` once a login actually lands,
/// so `false` means nobody is being sent to this port any more and the task can
/// free it instead of idling to the deadline.
fn attempt_pending(app: &AppHandle, nonce: &str) -> bool {
    let state = app.state::<crate::state::AppState>();
    let pending = state.pending_login.lock().unwrap();
    pending.as_ref().is_some_and(|p| p.nonce == nonce)
}

/// Read one HTTP request; if it's the OAuth callback *with a matching `state`*,
/// reply with a small "you can close this" page and return the decoded
/// `(access, refresh)` tokens. A request whose `state` doesn't match (a stray
/// probe, or a forged callback racing the real one) gets the wait page and
/// `None`, so the listener keeps waiting for the genuine redirect.
async fn serve(mut stream: TcpStream, expected_state: &str) -> Result<Option<(String, String)>> {
    // Request line + headers are tiny; one read of the first chunk is enough to
    // see `GET /callback?...`; we never need the (absent) body.
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).await.context("reading request")?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let Some(line) = head.lines().next() else {
        return Ok(None);
    };
    let accept_language = head.lines().find_map(|h| {
        let (name, value) = h.split_once(':')?;
        name.eq_ignore_ascii_case("accept-language")
            .then(|| value.trim())
    });
    // "GET /callback?access_token=…&refresh_token=…&state=… HTTP/1.1"
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let Some((_, query)) = target.split_once('?') else {
        respond(&mut stream, WAIT_PAGE).await?;
        return Ok(None);
    };

    match parse_callback(query, expected_state) {
        Some((access, refresh)) => {
            respond(&mut stream, &ok_page(accept_language)).await?;
            Ok(Some((access, refresh)))
        }
        None => {
            respond(&mut stream, WAIT_PAGE).await?;
            Ok(None)
        }
    }
}

/// Parse the callback query string and enforce the CSRF `state` nonce. Returns
/// the tokens only when the echoed `state` matches the one this login attempt
/// generated; any mismatch, missing state, or empty access token yields `None`.
/// Split out from [`serve`] so the security-critical check is unit-testable
/// without a live socket.
fn parse_callback(query: &str, expected_state: &str) -> Option<(String, String)> {
    let mut access = None;
    let mut refresh = String::new();
    let mut state: Option<String> = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            "access_token" => access = Some(percent_decode(v)),
            "refresh_token" => refresh = percent_decode(v),
            "state" => state = Some(percent_decode(v)),
            _ => {}
        }
    }

    // Fail closed: an empty expected nonce (shouldn't happen) or a non-matching
    // echoed state means we never hand back tokens.
    if expected_state.is_empty() || state.as_deref() != Some(expected_state) {
        if state.is_some() {
            tracing::warn!("loopback OAuth: rejected callback with mismatched state nonce");
        }
        return None;
    }

    match access {
        Some(a) if !a.is_empty() => Some((a, refresh)),
        _ => None,
    }
}

async fn respond(stream: &mut TcpStream, body: &str) -> Result<()> {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(resp.as_bytes())
        .await
        .context("writing response")?;
    let _ = stream.flush().await;
    Ok(())
}

/// Minimal percent-decoder for query values. The web side `encodeURIComponent`s
/// each token; JWT access tokens only contain URL-safe chars, but Supabase
/// refresh tokens can carry base64 `+`/`/` which arrive as `%2B`/`%2F`, so we
/// decode defensively. Note: `encodeURIComponent` never emits a bare `+`, so we
/// deliberately do NOT treat `+` as a space (that's form-encoding, not this).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_jwt_unchanged() {
        let jwt = "eyJhbGc.iOiJI-zI1_Ni..sig";
        assert_eq!(percent_decode(jwt), jwt);
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("a%2Bb%2Fc%3D"), "a+b/c=");
    }

    #[test]
    fn leaves_literal_plus_alone() {
        // encodeURIComponent never emits a bare '+', but if one slips through it
        // must stay a '+', not become a space.
        assert_eq!(percent_decode("a+b"), "a+b");
    }

    #[test]
    fn callback_accepts_matching_state() {
        let q = "access_token=tok&refresh_token=ref&state=abc123";
        assert_eq!(
            parse_callback(q, "abc123"),
            Some(("tok".to_string(), "ref".to_string()))
        );
    }

    #[test]
    fn callback_rejects_wrong_state() {
        let q = "access_token=tok&refresh_token=ref&state=evil";
        assert_eq!(parse_callback(q, "abc123"), None);
    }

    #[test]
    fn callback_rejects_missing_state() {
        // The pre-fix forged-callback shape: tokens with no state must not pass.
        let q = "access_token=tok&refresh_token=ref";
        assert_eq!(parse_callback(q, "abc123"), None);
    }

    #[test]
    fn the_return_page_follows_the_browser_language() {
        assert_eq!(return_message(Some("es-ES,es;q=0.9,en;q=0.8")).0, "es");
        assert_eq!(return_message(Some("pt-BR")).0, "pt");
        // A language we have no sentence for falls through to the next one.
        assert_eq!(return_message(Some("nl-NL,nl;q=0.9,de;q=0.8")).0, "de");
        assert_eq!(return_message(Some("nl")).0, "en");
        assert_eq!(return_message(None).0, "en");
    }

    /// Over a real socket: the header is read from the request, and the length is
    /// counted in bytes, which the accents and the CJK sentences depend on.
    #[tokio::test]
    async fn serves_the_callback_page_in_the_browser_language() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve(stream, "nonce").await.unwrap()
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(
                b"GET /callback?access_token=tok&refresh_token=ref&state=nonce HTTP/1.1\r\n\
                  Host: 127.0.0.1\r\nAccept-Language: ja,en;q=0.8\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();

        assert_eq!(
            server.await.unwrap(),
            Some(("tok".to_string(), "ref".to_string()))
        );
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(head.contains(&format!("Content-Length: {}", body.len())));
        assert!(body.contains("lang=\"ja\""));
        assert!(body.contains(return_sentence("ja").unwrap()));
    }

    #[test]
    fn callback_rejects_empty_expected_state() {
        let q = "access_token=tok&state=";
        assert_eq!(parse_callback(q, ""), None);
    }
}
