//! The raw queries, whose answers `convert` reads as raw JSON, and the mobile app's
//! `browse` request.

use std::borrow::Cow;

use ytmapi_rs::auth::{AuthToken, BrowserToken};
use ytmapi_rs::parse::{ParseFrom, ProcessedResult};
use ytmapi_rs::query::{GetMethod, GetQuery, PostMethod, PostQuery, Query};

use crate::ApiError;

/// The Home feed's first page (`FEmusic_home`), fetched as raw JSON.
pub(super) struct HomeQuery;

impl<A: AuthToken> Query<A> for HomeQuery {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl PostQuery for HomeQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::from_iter([("browseId".to_string(), "FEmusic_home".into())])
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "browse"
    }
}

impl ParseFrom<HomeQuery> for Unparsed {
    fn parse_from(_: ProcessedResult<HomeQuery>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// A `browse` page by id, fetched as raw JSON: Explore's pages (new releases, charts,
/// moods and genres, a mood's playlists) and the history. ytmapi-rs 0.3.3 has no charts
/// or new-releases query, and its history and moods parsers drop what the pages show.
pub(super) struct Browse<'a> {
    pub(super) id: &'a str,
    /// A mood or genre's playlists (`FEmusic_moods_and_genres_category`).
    pub(super) params: Option<&'a str>,
    /// The charts' country (`ZZ` is Global), as ytmusicapi's `get_charts` sends it.
    pub(super) country: Option<&'a str>,
}

impl<A: AuthToken> Query<A> for Browse<'_> {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl PostQuery for Browse<'_> {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut body = serde_json::Map::from_iter([("browseId".to_string(), self.id.into())]);
        if let Some(p) = self.params {
            body.insert("params".into(), p.into());
        }
        if let Some(c) = self.country {
            body.insert(
                "formData".into(),
                serde_json::json!({ "selectedValues": [c] }),
            );
        }
        body
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "browse"
    }
}

impl ParseFrom<Browse<'_>> for Unparsed {
    fn parse_from(_: ProcessedResult<Browse<'_>>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// The mobile app's client, as ytmusicapi's `as_mobile` sends it: YouTube Music gives
/// timed lyrics (`timedLyricsData`) to it and not to the web client.
const MOBILE_CLIENT: &str = "ANDROID_MUSIC";
const MOBILE_CLIENT_VERSION: &str = "7.21.50";

/// The `browse` endpoint, for the mobile client's request.
pub(super) const MOBILE_BROWSE_URL: &str =
    "https://music.youtube.com/youtubei/v1/browse?alt=json&prettyPrint=false";
pub(super) const YTM_ORIGIN: &str = "https://music.youtube.com";
/// A desktop browser's, as ytmusicapi sends with the mobile client.
pub(super) const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// The body of a `browse` request for page `id` as the mobile app.
pub(super) fn mobile_browse_body(id: &str) -> serde_json::Value {
    serde_json::json!({
        "context": {
            "client": {
                "clientName": MOBILE_CLIENT,
                "clientVersion": MOBILE_CLIENT_VERSION,
                "hl": "en",
            },
            "user": {},
        },
        "browseId": id,
    })
}

/// A `browse` page by id as the mobile app asks for it, raw, from `url`: the Lyrics tab's page
/// with timed lines. Sent over `public`, without the session: with the web session's cookies
/// the mobile client is answered 400 (checked live 2026-10-31, as in ytmusicapi issue 828),
/// and the page is public. One request, read-only.
pub(super) async fn mobile_browse(
    public: &reqwest::Client,
    url: &str,
    id: &str,
) -> Result<String, ApiError> {
    let network = |e: reqwest::Error| ApiError::Network(e.to_string());
    let response = public
        .post(url)
        .header(reqwest::header::ORIGIN, YTM_ORIGIN)
        .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT)
        .json(&mobile_browse_body(id))
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::Network(format!(
            "mobile lyrics page: status {status}"
        )));
    }
    response.text().await.map_err(network)
}

/// The account menu (`account/account_menu`), fetched as raw JSON.
pub(super) struct AccountMenuQuery;

impl PostQuery for AccountMenuQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "account/account_menu"
    }
}

/// The account switcher, fetched as raw JSON: the GET behind the account menu's
/// "Switch account" (`getAccountSwitcherEndpoint`). Innertube's `account/accounts_list`
/// answers the music client with no accounts.
pub(super) struct AccountSwitcherQuery;

impl Query<BrowserToken> for AccountSwitcherQuery {
    type Output = Unparsed;
    type Method = GetMethod;
}

impl GetQuery for AccountSwitcherQuery {
    fn url(&self) -> &str {
        "https://music.youtube.com/getAccountSwitcherEndpoint"
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
}

impl ParseFrom<AccountSwitcherQuery> for Unparsed {
    fn parse_from(_: ProcessedResult<AccountSwitcherQuery>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// A POST query as it goes out: its path, body and URL parameters, read from a typed query
/// once (`of`). Every raw request is sent as one, so the request code (both tokens, the
/// resumed session's retry) is compiled once, not once for each query type. It answers for
/// either token: `raw`'s bounds keep an account-only query off the guest path, and the
/// callers of `raw_post` pass catalogue queries only (`raw_account` refuses a guest).
#[derive(Clone)]
pub(super) struct Post<'q> {
    path: &'q str,
    header: serde_json::Map<String, serde_json::Value>,
    params: Vec<(&'q str, Cow<'q, str>)>,
}

impl<'q> Post<'q> {
    pub(super) fn of(query: &'q impl PostQuery) -> Post<'q> {
        Post {
            path: query.path(),
            header: query.header(),
            params: query.params(),
        }
    }

    /// The next page of this list, by its continuation `token`: the same request shape as
    /// ytmapi-rs's own continuation query, for lists its typed parser can't read.
    pub(super) fn next(&self, token: String) -> Post<'q> {
        Post {
            path: self.path,
            header: self.header.clone(),
            params: vec![
                ("ctoken", token.clone().into()),
                ("continuation", token.into()),
            ],
        }
    }
}

#[derive(Debug)]
pub(super) struct Unparsed;

impl<A: AuthToken> Query<A> for Post<'_> {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl PostQuery for Post<'_> {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        self.header.clone()
    }
    fn params(&self) -> Vec<(&str, Cow<'_, str>)> {
        self.params
            .iter()
            .map(|(name, value)| (*name, Cow::Borrowed(value.as_ref())))
            .collect()
    }
    fn path(&self) -> &str {
        self.path
    }
}

impl ParseFrom<Post<'_>> for Unparsed {
    fn parse_from(_: ProcessedResult<Post<'_>>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}
