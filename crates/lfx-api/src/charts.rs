//! Client für den optionalen Charts-Worker (Verträge `docs/charts-api.md` und `docs/stats-api.md` des Web-Themes).
//!
//! - Die Basis-URL kommt aus dem Branding-CSS ([`crate::branding::Branding::charts_url`]), nie fest verdrahtet.
//! - Das Jellyfin-Token geht ausschließlich im Header `X-Emby-Token` an den Worker, nie in die URL.
//! - Weiterleitungen werden nicht verfolgt (das Token darf nirgends sonst hin).

use crate::branding::{Branding, clean_charts_url};
use crate::client::Client;
use crate::error::{ApiError, Result};
use crate::redact::redact_url;
use crate::secret::Secret;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// Eintrag der Charts.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ChartEntry {
    /// Element-Id (Film oder Serie).
    pub id: String,
    /// `Movie` oder `Series` (nur in `all`).
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Verschiedene Zuschauer im Zeitraum.
    pub viewers: i64,
    /// Wiedergaben.
    pub plays: i64,
    /// Gesamtdauer in Sekunden.
    pub seconds: i64,
}

/// Antwort von `GET /charts`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Charts {
    /// Vertragsversion.
    pub version: i64,
    /// Berechnet am (ISO-8601).
    pub generated_at: String,
    /// Fenster in Tagen.
    pub window_days: i64,
    /// Sortiermaß (`viewers`).
    pub metric: String,
    /// Beliebteste Filme.
    pub movies: Vec<ChartEntry>,
    /// Beliebteste Serien.
    pub series: Vec<ChartEntry>,
    /// Gemischt.
    pub all: Vec<ChartEntry>,
}

/// Zeitraum der Statistik.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatsRange {
    /// Letzte 7 Tage.
    Days7,
    /// Letzte 30 Tage (Standard).
    #[default]
    Days30,
    /// Laufendes Kalenderjahr.
    Year,
    /// Gesamte Zeit.
    All,
}

impl StatsRange {
    /// Wert des Query-Parameters `range`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Days7 => "7d",
            Self::Days30 => "30d",
            Self::Year => "year",
            Self::All => "all",
        }
    }
}

/// Summen der Statistik.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsTotals {
    /// Gesamtdauer in Sekunden.
    pub seconds: i64,
    /// Wiedergaben.
    pub plays: i64,
    /// Gezählte Folgen.
    pub episodes: i64,
    /// Gezählte Filme.
    pub movies: i64,
    /// Verschiedene Serien.
    pub series: i64,
    /// Aktive Tage.
    pub active_days: i64,
    /// Erste Wiedergabe.
    pub first_at: Option<String>,
    /// Letzte Wiedergabe.
    pub last_at: Option<String>,
}

/// Top-Eintrag (Serie oder Film).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsTop {
    /// Element-Id.
    pub id: String,
    /// Dauer in Sekunden.
    pub seconds: i64,
    /// Folgen (Serien).
    pub episodes: Option<i64>,
    /// Wiedergaben (Filme).
    pub plays: Option<i64>,
}

/// Ein Zeitabschnitt des Verlaufs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsBucket {
    /// Schlüssel (`YYYY-MM-DD`, `YYYY-WW`, `YYYY-MM`).
    pub key: String,
    /// Dauer in Sekunden.
    pub seconds: i64,
}

/// Verlauf.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsTimeline {
    /// `day`, `week` oder `month`.
    pub unit: String,
    /// Abschnitte mit Daten.
    pub buckets: Vec<StatsBucket>,
}

/// Zelle der Wochen-/Stunden-Karte.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsHeat {
    /// Wochentag, 0 = Sonntag.
    pub dow: u8,
    /// Stunde 0–23.
    pub hour: u8,
    /// Dauer in Sekunden.
    pub seconds: i64,
}

/// Gerät.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsDevice {
    /// Programm.
    pub client: String,
    /// Gerät.
    pub device: String,
    /// Wiedergaben.
    pub plays: i64,
    /// Dauer in Sekunden.
    pub seconds: i64,
}

/// Wiedergaben und Dauer je Methode.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsMethod {
    /// Wiedergaben.
    pub plays: i64,
    /// Dauer in Sekunden.
    pub seconds: i64,
}

/// Wiedergabemethoden.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsMethods {
    /// Direkt abgespielt.
    pub direct_play: StatsMethod,
    /// Direkt gestreamt (auch Remux).
    pub direct_stream: StatsMethod,
    /// Umgewandelt.
    pub transcode: StatsMethod,
    /// Sonstige.
    pub other: StatsMethod,
}

/// Vergleich mit anderen (nur Rang und Durchschnitt).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsCompare {
    /// Rang (1 = meiste Zeit).
    pub rank: i64,
    /// Anzahl aktiver Benutzer.
    pub of: i64,
    /// Durchschnittliche Dauer in Sekunden.
    pub avg_seconds: i64,
}

/// Tag mit der längsten Dauer.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsTopDay {
    /// Datum.
    pub date: String,
    /// Dauer in Sekunden.
    pub seconds: i64,
}

/// Serie mit den meisten Folgen an einem Tag.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsSeriesDay {
    /// Datum.
    pub date: String,
    /// Serien-Id.
    pub series_id: String,
    /// Folgen.
    pub episodes: i64,
}

/// Marathon-Werte.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsBinge {
    /// Längster Tag.
    pub top_day: Option<StatsTopDay>,
    /// Meiste Folgen einer Serie an einem Tag.
    pub series_day: Option<StatsSeriesDay>,
}

/// Antwort von `GET /me/stats`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeStats {
    /// Vertragsversion.
    pub version: i64,
    /// Zeitraum.
    pub range: String,
    /// Berechnet am (ISO-8601).
    pub computed_at: String,
    /// Veralteter Stand?
    pub stale: bool,
    /// Näherungswerte?
    pub approx: bool,
    /// Benutzer steht auf der Ignorierliste?
    pub ignored: bool,
    /// Summen.
    pub totals: StatsTotals,
    /// Meistgesehene Serien.
    pub top_series: Vec<StatsTop>,
    /// Meistgesehene Filme.
    pub top_movies: Vec<StatsTop>,
    /// Verlauf.
    pub timeline: StatsTimeline,
    /// Wochentag/Stunde-Karte.
    pub heatmap: Vec<StatsHeat>,
    /// Marathon-Werte.
    pub binge: StatsBinge,
    /// Geräte.
    pub devices: Vec<StatsDevice>,
    /// Methoden.
    pub methods: StatsMethods,
    /// Vergleich (fehlt bei zu wenigen Benutzern).
    pub compare: Option<StatsCompare>,
}

/// Client des Charts-Workers.
#[derive(Clone)]
pub struct ChartsClient {
    http: reqwest::Client,
    base: String,
    token: Secret,
}

impl std::fmt::Debug for ChartsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartsClient")
            .field("base", &"<url>")
            .field("token", &self.token)
            .finish()
    }
}

impl ChartsClient {
    /// Neuer Client. `base_url` wird wie im Web-Theme bereinigt (https, Loopback-http nur zum Testen).
    pub fn new(base_url: &str, token: Secret) -> Result<Self> {
        let base =
            clean_charts_url(base_url).ok_or_else(|| ApiError::InvalidUrl("Charts".into()))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!(
                "{}/{}",
                crate::CLIENT_NAME,
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(ApiError::from_reqwest)?;
        Ok(Self { http, base, token })
    }

    /// Client aus Jellyfin-Sitzung und Branding; `None`, wenn Charts nicht konfiguriert oder nicht angemeldet.
    pub fn from_branding(client: &Client, branding: &Branding) -> Option<Result<Self>> {
        let url = branding.charts_url.as_deref()?;
        let session = client.session()?;
        Some(Self::new(url, session.token))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.base))
            .map_err(|_| ApiError::InvalidUrl("Charts".into()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        let started = std::time::Instant::now();
        let resp = self
            .http
            .get(url.clone())
            .header("X-Emby-Token", self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = resp.status();
        tracing::debug!(url = %redact_url(url.as_str()), status = status.as_u16(), ms = started.elapsed().as_millis() as u64, "Charts-Antwort");
        match status {
            s if s.is_success() => {
                let bytes = resp.bytes().await.map_err(ApiError::from_reqwest)?;
                serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ApiError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ApiError::NotFound),
            s => Err(ApiError::Status {
                status: s.as_u16(),
                path: path.to_owned(),
            }),
        }
    }

    /// Serverweite Charts (`GET /charts`).
    pub async fn charts(&self) -> Result<Charts> {
        self.get("/charts", &[]).await
    }

    /// Eigene Statistik (`GET /me/stats?range=…`).
    pub async fn my_stats(&self, range: StatsRange) -> Result<MeStats> {
        self.get("/me/stats", &[("range", range.as_str())]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_insecure_base() {
        assert!(ChartsClient::new("http://charts.example.invalid", Secret::new("t")).is_err());
        assert!(ChartsClient::new("https://charts.example.invalid", Secret::new("t")).is_ok());
    }

    #[test]
    fn debug_hides_token_and_url() {
        let c = ChartsClient::new("https://charts.example.invalid", Secret::new("geheim")).unwrap();
        let text = format!("{c:?}");
        assert!(!text.contains("geheim") && !text.contains("example"));
    }

    #[test]
    fn decodes_charts_shape() {
        let json = r#"{"version":1,"generatedAt":"2026-01-01T00:00:00Z","windowDays":30,"metric":"viewers",
          "movies":[{"id":"aa","viewers":3,"plays":5,"seconds":18000}],
          "series":[{"id":"bb","viewers":2,"plays":14,"seconds":36000}],
          "all":[{"id":"aa","type":"Movie","viewers":3,"plays":5,"seconds":18000}]}"#;
        let c: Charts = serde_json::from_str(json).unwrap();
        assert_eq!(c.window_days, 30);
        assert_eq!(c.all[0].kind.as_deref(), Some("Movie"));
        assert_eq!(c.movies[0].seconds, 18000);
    }

    #[test]
    fn decodes_stats_shape() {
        let json = r#"{"version":1,"range":"30d","computedAt":"2026-01-01T00:00:00Z","stale":false,"approx":false,"ignored":false,
          "totals":{"seconds":1200,"plays":3,"episodes":2,"movies":1,"series":1,"activeDays":2,"firstAt":"2026-01-01 20:00:00","lastAt":"2026-01-02 21:00:00"},
          "topSeries":[{"id":"s1","seconds":900,"episodes":2}],"topMovies":[{"id":"m1","seconds":300,"plays":1}],
          "timeline":{"unit":"day","buckets":[{"key":"2026-01-01","seconds":600}]},
          "heatmap":[{"dow":0,"hour":21,"seconds":600}],
          "binge":{"topDay":{"date":"2026-01-01","seconds":600},"seriesDay":{"date":"2026-01-01","seriesId":"s1","episodes":2}},
          "devices":[{"client":"Test","device":"PC","plays":3,"seconds":1200}],
          "methods":{"directPlay":{"plays":2,"seconds":800},"directStream":{"plays":0,"seconds":0},"transcode":{"plays":1,"seconds":400},"other":{"plays":0,"seconds":0}},
          "compare":{"rank":2,"of":4,"avgSeconds":1500}}"#;
        let s: MeStats = serde_json::from_str(json).unwrap();
        assert_eq!(s.totals.active_days, 2);
        assert_eq!(s.binge.series_day.unwrap().series_id, "s1");
        assert_eq!(s.compare.unwrap().rank, 2);
        assert_eq!(s.methods.transcode.plays, 1);
        let none: MeStats = serde_json::from_str(r#"{"range":"all","compare":null}"#).unwrap();
        assert!(none.compare.is_none());
    }
}
