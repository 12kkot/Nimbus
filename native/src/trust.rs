//! Which voice servers Nimbus may connect to. A community's address sits in a channel's topic
//! (`[nimbus-stream] address`), so anyone who can edit a topic can point us anywhere: whoever
//! runs that server sees the IP address of everyone who joins. So an address we haven't seen for
//! this server is asked about first, the answer is remembered, and addresses on the open internet
//! must be encrypted (https, and wss for LiveKit).

use std::net::Ipv4Addr;

use serde_json::{json, Value};

/// The port of Nimbus's own token service (see `DEFAULT_SERVER`).
const TOKEN_PORT: u16 = 7890;

/// An address read from a topic: `http://host:port` or `https://host`.
#[derive(Debug, Clone, PartialEq)]
pub struct Addr {
    pub https: bool,
    pub host: String,
    pub port: Option<u16>,
}

impl Addr {
    /// The same address written one way, for remembering answers.
    pub fn normal(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        match self.port {
            Some(p) => format!("{scheme}://{}:{p}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }
}

/// Reads an address; anything but http and https is refused.
pub fn parse(text: &str) -> Result<Addr, String> {
    let text = text.trim();
    let (https, rest) = if let Some(r) = text.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = text.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(crate::i18n::tf("адрес «{}» должен начинаться с http:// или https://", &[&text]));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return Err(crate::i18n::tf("в адресе «{}» не получилось найти сервер", &[&text]));
    }
    // [::1]:7890, host:7890, host
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (h, after) = inner.split_once(']').ok_or_else(|| crate::i18n::tf("в адресе «{}» не закрыта ]", &[&text]))?;
        (h.to_string(), after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    let port = match port {
        Some(p) => Some(p.parse::<u16>().map_err(|_| crate::i18n::tf("в адресе «{}» неверный порт «{}»", &[&text, &p]))?),
        None => None,
    };
    if host.is_empty() {
        return Err(crate::i18n::tf("в адресе «{}» нет имени сервера", &[&text]));
    }
    Ok(Addr {
        https,
        host: host.to_lowercase(),
        port,
    })
}

/// This computer itself.
pub fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// Addresses that aren't on the open internet: this computer, home and office networks, and
/// Tailscale (its 100.64.0.0/10 addresses and *.ts.net names; its tunnel is encrypted).
pub fn is_private(host: &str) -> bool {
    if is_loopback(host) || host.ends_with(".ts.net") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<Ipv4Addr>() {
        Ok(ip) => {
            let o = ip.octets();
            ip.is_private() || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        Err(_) => false,
    }
}

/// Plain http is only for private addresses; the open internet needs https.
pub fn check_transport(addr: &Addr) -> Result<(), String> {
    if addr.https || is_private(&addr.host) {
        Ok(())
    } else {
        Err(crate::i18n::tf(
            "Сервер {} стоит в интернете, но без шифрования (http). Ему нужен https: через Caddy, Cloudflare Tunnel или Tailscale (см. server\\README.md).",
            &[&addr.host],
        ))
    }
}

/// The LiveKit address the token service returned: on the open internet only wss.
pub fn check_ws(url: &str, addr: &Addr) -> Result<(), String> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("wss://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("ws://") {
        (false, r)
    } else {
        return Err(crate::i18n::tf("сервер вернул странный адрес голоса «{}»", &[&url]));
    };
    let host = rest.split(['/', ':']).next().unwrap_or("").to_lowercase();
    if secure || (is_private(&host) && is_private(&addr.host)) {
        Ok(())
    } else {
        Err(crate::i18n::tf(
            "Сервер {} отдал голос без шифрования (ws://{}). Нужен wss://: настрой https для LiveKit (server\\README.md).",
            &[&addr.host, &host],
        ))
    }
}

/// What to do before connecting.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Fine: our own computer, or already allowed.
    Connect,
    /// Ask first: we haven't heard an answer for this server and address.
    Ask,
    /// Said no before.
    Refuse,
}

/// The key under which the answer for a server's address is kept.
pub fn key(guild: &str, addr: &Addr) -> String {
    format!("{guild}|{}", addr.normal())
}

/// `answers` is the saved map { "server|address": true/false }.
pub fn decide(guild: &str, addr: &Addr, answers: &Value) -> Decision {
    // Only the token service's own port on this computer needs no question; any other local
    // port is something else's, and a topic could name it.
    if is_loopback(&addr.host) && addr.port == Some(TOKEN_PORT) {
        return Decision::Connect;
    }
    match answers[key(guild, addr)].as_bool() {
        Some(true) => Decision::Connect,
        Some(false) => Decision::Refuse,
        None => Decision::Ask,
    }
}

/// The map with one more answer.
pub fn with_answer(answers: &Value, guild: &str, addr: &Addr, allowed: bool) -> Value {
    let mut map = if answers.is_object() { answers.clone() } else { json!({}) };
    map[key(guild, addr)] = json!(allowed);
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> Addr {
        parse(s).unwrap()
    }

    #[test]
    fn addresses_are_read() {
        assert_eq!(
            addr("http://localhost:7890"),
            Addr {
                https: false,
                host: "localhost".into(),
                port: Some(7890)
            }
        );
        assert_eq!(
            addr("https://Voice.Example.com/"),
            Addr {
                https: true,
                host: "voice.example.com".into(),
                port: None
            }
        );
        assert_eq!(addr("http://[::1]:7890").host, "::1");
        assert_eq!(addr("http://192.168.1.5:7890/token").normal(), "http://192.168.1.5:7890");
        for bad in [
            "",
            "ftp://x.test",
            "voice.example.com",
            "http://",
            "http://user@x.test",
            "http://x.test:99999",
            "http://x.test:abc",
            "http://a b",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn private_and_public_hosts() {
        for h in [
            "localhost",
            "127.0.0.1",
            "::1",
            "192.168.0.10",
            "10.1.2.3",
            "172.16.4.4",
            "100.64.0.1",
            "100.100.5.5",
            "my-pc.tail1234.ts.net",
        ] {
            assert!(is_private(h), "{h} is private");
        }
        for h in ["8.8.8.8", "100.128.0.1", "172.32.0.1", "example.com", "11.0.0.1", "192.169.0.1"] {
            assert!(!is_private(h), "{h} is public");
        }
        assert!(is_loopback("127.5.5.5") && !is_loopback("192.168.0.1"));
    }

    #[test]
    fn plain_http_only_for_private_addresses() {
        assert!(check_transport(&addr("http://192.168.1.5:7890")).is_ok());
        assert!(check_transport(&addr("http://100.101.102.103:7890")).is_ok());
        assert!(check_transport(&addr("https://voice.example.com")).is_ok());
        let err = check_transport(&addr("http://voice.example.com:7890")).unwrap_err();
        assert!(err.contains("https"), "{err}");
        assert!(check_transport(&addr("http://8.8.8.8:7890")).is_err());
    }

    #[test]
    fn livekit_address_must_be_encrypted_on_the_internet() {
        let public = addr("https://voice.example.com");
        assert!(check_ws("wss://voice.example.com", &public).is_ok());
        assert!(check_ws("ws://voice.example.com:7880", &public).is_err());
        let home = addr("http://192.168.1.5:7890");
        assert!(check_ws("ws://192.168.1.5:7880", &home).is_ok());
        // A private server must not send us to the open internet in the clear.
        assert!(check_ws("ws://example.org:7880", &home).is_err());
        assert!(check_ws("http://x", &home).is_err());
    }

    #[test]
    fn ask_once_then_remember() {
        let a = addr("https://voice.example.com");
        let none = json!({});
        assert_eq!(decide("1", &a, &none), Decision::Ask);
        let yes = with_answer(&none, "1", &a, true);
        assert_eq!(decide("1", &a, &yes), Decision::Connect);
        assert_eq!(decide("2", &a, &yes), Decision::Ask, "another server asks again");
        assert_eq!(
            decide("1", &addr("https://other.example.com"), &yes),
            Decision::Ask,
            "another address asks again"
        );
        let no = with_answer(&yes, "1", &a, false);
        assert_eq!(decide("1", &a, &no), Decision::Refuse);
        // This computer never asks, and a damaged map just means asking.
        assert_eq!(decide("1", &addr("http://localhost:7890"), &none), Decision::Connect);
        // ...but only at the token service's port: another local port is somebody else's service.
        assert_eq!(decide("1", &addr("http://localhost:9200"), &none), Decision::Ask);
        assert_eq!(decide("1", &addr("http://127.0.0.1"), &none), Decision::Ask);
        assert_eq!(decide("1", &a, &json!("garbage")), Decision::Ask);
        assert_eq!(with_answer(&json!(null), "1", &a, true)["1|https://voice.example.com"], json!(true));
    }
}
