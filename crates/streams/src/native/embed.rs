//! The embed page's client config, and the page kept in `{home}/resolver/state.json`.

use std::time::UNIX_EPOCH;

use serde_json::{Value, json};

use super::players::{installed, write_atomic};
use super::{CLIENT_VERSION, EmbedConfig, Inner, Ready};

impl Inner {
    /// The saved page's config, and the whole saved state.
    pub(super) fn saved(&self) -> Option<(EmbedConfig, Value)> {
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(self.dir.join("state.json")).ok()?)
                .ok()?;
        let config = EmbedConfig {
            client_version: v["client_version"].as_str()?.to_string(),
            visitor_data: v["visitor_data"].as_str().map(str::to_string),
            host_flags: v["host_flags"].as_str().map(str::to_string),
            player: v["player"].as_str()?.to_string(),
        };
        Some((config, v))
    }

    pub(super) fn load(&self) -> Option<Ready> {
        let (config, v) = self.saved()?;
        let player = installed(&self.dir.join("players"), v["active"].as_str()?)?;
        Some(Ready {
            config,
            player,
            fetched: crate::expiry::unix_secs(v["fetched"].as_u64()?)?,
            page: self.next_page(),
        })
    }

    pub(super) fn save(&self, r: &Ready) {
        let v = json!({
            "client_version": r.config.client_version,
            "visitor_data": r.config.visitor_data,
            "host_flags": r.config.host_flags,
            "player": r.config.player,
            "active": r.player.id,
            "fetched": r.fetched.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        });
        let _ = std::fs::create_dir_all(&self.dir)
            .and_then(|()| write_atomic(&self.dir.join("state.json"), v.to_string().as_bytes()));
    }
}

/// The client config in an embed page (`https://www.youtube.com/embed/{id}?html5=1`).
pub(crate) fn parse_embed_page(html: &str) -> Option<EmbedConfig> {
    let player = html.split("/s/player/").skip(1).find_map(|rest| {
        let id = rest.split('/').next()?;
        valid_player_id(id).then(|| id.to_string())
    })?;
    let client_version = json_string(html, "INNERTUBE_CONTEXT_CLIENT_VERSION")
        .or_else(|| json_string(html, "INNERTUBE_CLIENT_VERSION"))
        .filter(|v| v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
        .unwrap_or_else(|| CLIENT_VERSION.to_string());
    Some(EmbedConfig {
        client_version,
        visitor_data: json_string(html, "VISITOR_DATA"),
        host_flags: json_string(html, "encryptedHostFlags"),
        player,
    })
}

pub(super) fn valid_player_id(id: &str) -> bool {
    (4..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The first `"key":"value"` string in `text`, unescaped.
fn json_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\":\"");
    let start = text.find(&pattern)? + pattern.len();
    let rest = &text[start..];
    let mut escaped = false;
    let end = rest.char_indices().find_map(|(i, c)| {
        let close = c == '"' && !escaped;
        escaped = c == '\\' && !escaped;
        close.then_some(i)
    })?;
    let s: String = serde_json::from_str(&format!("\"{}\"", &rest[..end])).ok()?;
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tests::config;

    #[test]
    fn the_embed_page_gives_the_client_config() {
        assert_eq!(
            config(),
            EmbedConfig {
                client_version: "2.20261028.01.00".into(),
                visitor_data: Some("CgtTY3J1YmJlZA%3D%3D".into()),
                host_flags: Some("AEZHscrubbed==".into()),
                player: "096efeec".into(),
            }
        );
        // A page with only the player: yt-dlp's client version, nothing else.
        let bare = parse_embed_page(r#"<script src="/s/player/abcd1234/base.js">"#).unwrap();
        assert_eq!(bare.client_version, CLIENT_VERSION);
        assert_eq!((bare.visitor_data, bare.host_flags), (None, None));
        assert_eq!(parse_embed_page("<html>no player</html>"), None);
        assert_eq!(
            parse_embed_page("/s/player/../../x/base.js"),
            None,
            "not an id"
        );
    }
}
