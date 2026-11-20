//! Weitere Bibliotheks-Endpunkte: Suche, Personen/Genres/Studios, Filter, Extras, Musik, Fotos, Bücher,
//! Wiedergabelisten. Alle Adressen ohne Token; Medien laden mpv/der Bildlader mit dem Auth-Header.

use crate::client::Client;
use crate::dto::*;
use crate::dto_ext::*;
use crate::endpoints::{NO_BODY, enc, opt, q};
use crate::error::Result;
use crate::query::{ItemsQuery, SortBy, SortOrder};
use reqwest::Method;
use serde::Serialize;

/// Optionen für [`Client::search_hints`].
#[derive(Debug, Clone)]
pub struct SearchHintOptions {
    /// Höchstzahl Treffer.
    pub limit: u32,
    /// Nur diese Elementarten.
    pub include_types: Vec<ItemKind>,
    /// Personen einbeziehen?
    pub include_people: bool,
    /// Medien einbeziehen?
    pub include_media: bool,
    /// Genres einbeziehen?
    pub include_genres: bool,
    /// Studios einbeziehen?
    pub include_studios: bool,
    /// Künstler einbeziehen?
    pub include_artists: bool,
    /// Nur unterhalb dieser Bibliothek.
    pub parent_id: Option<String>,
}

impl Default for SearchHintOptions {
    fn default() -> Self {
        Self {
            limit: 24,
            include_types: Vec::new(),
            include_people: true,
            include_media: true,
            include_genres: true,
            include_studios: true,
            include_artists: true,
            parent_id: None,
        }
    }
}

/// Änderung der Benutzerdaten (`POST /UserItems/{id}/UserData`); nur gesetzte Felder werden gesendet.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserDataUpdate {
    /// Position in Ticks (bei Büchern: Lesefortschritt nach Wahl der App).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_position_ticks: Option<i64>,
    /// Gesehen/gelesen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played: Option<bool>,
    /// Favorit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    /// Gefällt/gefällt nicht.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<bool>,
    /// Anzahl Wiedergaben.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_count: Option<i64>,
}

/// Optionen der Audio-Stream-URL ([`Client::audio_stream_url`]).
#[derive(Debug, Clone)]
pub struct AudioStreamOptions {
    /// Container, die der Player direkt abspielt.
    pub containers: Vec<String>,
    /// Zielcodec bei Umwandlung.
    pub audio_codec: String,
    /// Zielcontainer bei Umwandlung.
    pub transcoding_container: String,
    /// Zielprotokoll bei Umwandlung (`hls`, `http`).
    pub transcoding_protocol: String,
    /// Höchste Bitrate in Bit/s.
    pub max_streaming_bitrate: u64,
    /// Startposition in Ticks.
    pub start_time_ticks: Option<i64>,
}

impl Default for AudioStreamOptions {
    fn default() -> Self {
        Self {
            containers: [
                "mp3", "aac", "m4a", "m4b", "flac", "wav", "ogg", "opus", "webma",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            audio_codec: "aac".to_owned(),
            transcoding_container: "ts".to_owned(),
            transcoding_protocol: "hls".to_owned(),
            max_streaming_bitrate: 140_000_000,
            start_time_ticks: None,
        }
    }
}

fn fields_opt(extra: &mut Vec<(String, String)>, fields: &[&str]) {
    opt(
        extra,
        "fields",
        (!fields.is_empty()).then(|| fields.join(",")),
    );
}

impl Client {
    // ------------------------------------------------------------ Server, Anmeldeseite

    /// Öffentliche Benutzerliste („Wer schaut?“), ohne Anmeldung. Leer, wenn der Server sie verbirgt.
    pub async fn public_users(&self) -> Result<Vec<UserDto>> {
        self.get_json("/Users/Public", &[]).await
    }

    /// Erreichbarkeitstest (`/System/Ping`), ohne Anmeldung.
    pub async fn ping(&self) -> Result<()> {
        self.get_text("/System/Ping", &[]).await.map(|_| ())
    }

    /// Beendet die Umwandlungsaufträge dieses Geräts und dieser Wiedergabesitzung
    /// (`DELETE /Videos/ActiveEncodings`). Erst den Player leeren, dann aufrufen, sonst startet der Job neu.
    pub async fn delete_active_encodings(&self, play_session_id: &str) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            "/Videos/ActiveEncodings",
            &q(&[
                ("deviceId", self.info().device_id.clone()),
                ("playSessionId", play_session_id.to_owned()),
            ]),
            NO_BODY,
        )
        .await
    }

    // ------------------------------------------------------------ Suche, Filter

    /// Suchvorschläge (`/Search/Hints`): Medien, Personen, Genres, Studios, Künstler.
    pub async fn search_hints(
        &self,
        term: &str,
        options: &SearchHintOptions,
    ) -> Result<SearchHintResult> {
        let mut extra = vec![
            ("searchTerm".to_owned(), term.to_owned()),
            ("limit".to_owned(), options.limit.to_string()),
            (
                "includePeople".to_owned(),
                options.include_people.to_string(),
            ),
            ("includeMedia".to_owned(), options.include_media.to_string()),
            (
                "includeGenres".to_owned(),
                options.include_genres.to_string(),
            ),
            (
                "includeStudios".to_owned(),
                options.include_studios.to_string(),
            ),
            (
                "includeArtists".to_owned(),
                options.include_artists.to_string(),
            ),
        ];
        if !options.include_types.is_empty() {
            extra.push((
                "includeItemTypes".to_owned(),
                options
                    .include_types
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        opt(&mut extra, "parentId", options.parent_id.clone());
        self.get_json("/Search/Hints", &self.with_user(&extra)?)
            .await
    }

    /// Personen (`/Persons`), z. B. mit `ItemsQuery::search_term`, `name_starts_with`, `limit`.
    pub async fn persons(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/Persons", &self.with_user(query.build())?)
            .await
    }

    /// Genres mit Filtern (`/Genres`): Bibliothek (`parent_id`), Elementarten, Suche, Buchstabe, Sortierung.
    pub async fn genres_filtered(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/Genres", &self.with_user(query.build())?)
            .await
    }

    /// Musikgenres (`/MusicGenres`).
    pub async fn music_genres(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/MusicGenres", &self.with_user(query.build())?)
            .await
    }

    /// Studios (`/Studios`).
    pub async fn studios(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/Studios", &self.with_user(query.build())?)
            .await
    }

    /// Mögliche Filterwerte einer Bibliothek (`/Items/Filters2`): Genres, Schlagwörter, Sprachen.
    pub async fn item_filters(
        &self,
        parent_id: Option<&str>,
        kinds: &[ItemKind],
        recursive: bool,
    ) -> Result<QueryFilters> {
        let mut extra = vec![("recursive".to_owned(), recursive.to_string())];
        opt(&mut extra, "parentId", parent_id.map(str::to_owned));
        if !kinds.is_empty() {
            extra.push((
                "includeItemTypes".to_owned(),
                kinds
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        self.get_json("/Items/Filters2", &self.with_user(&extra)?)
            .await
    }

    /// Extras (Making-of, Interviews, ...) eines Elements.
    pub async fn special_features(&self, id: &str) -> Result<Vec<BaseItem>> {
        self.get_json(
            &format!("/Items/{}/SpecialFeatures", enc(id)),
            &self.with_user(&[])?,
        )
        .await
    }

    /// Lokale Trailer eines Elements.
    pub async fn local_trailers(&self, id: &str) -> Result<Vec<BaseItem>> {
        self.get_json(
            &format!("/Items/{}/LocalTrailers", enc(id)),
            &self.with_user(&[])?,
        )
        .await
    }

    /// Vorspann-Elemente (Intros) vor einem Titel.
    pub async fn intros(&self, id: &str) -> Result<QueryResult<BaseItem>> {
        self.get_json(&format!("/Items/{}/Intros", enc(id)), &self.with_user(&[])?)
            .await
    }

    /// Anzahl Elemente je Art (`/Items/Counts`), optional nur Favoriten.
    pub async fn item_counts(&self, favorites_only: bool) -> Result<ItemCounts> {
        let extra = if favorites_only {
            vec![("isFavorite".to_owned(), "true".to_owned())]
        } else {
            Vec::new()
        };
        self.get_json("/Items/Counts", &self.with_user(&extra)?)
            .await
    }

    /// Benutzerdaten setzen (`POST /UserItems/{id}/UserData`), z. B. Lesefortschritt eines Buchs.
    pub async fn update_user_data(
        &self,
        id: &str,
        update: &UserDataUpdate,
    ) -> Result<UserItemData> {
        self.call_json(
            Method::POST,
            &format!("/UserItems/{}/UserData", enc(id)),
            &self.with_user(&[])?,
            Some(update),
        )
        .await
    }

    /// Löscht ein Element auf dem Server (`DELETE /Items/{id}`, z. B. eine Wiedergabeliste; braucht Recht).
    pub async fn delete_item(&self, id: &str) -> Result<()> {
        self.call_empty(Method::DELETE, &format!("/Items/{}", enc(id)), &[], NO_BODY)
            .await
    }

    /// Download-Adresse der Originaldatei (Buch, Foto, Titel) **ohne Token**; laden mit
    /// [`Client::fetch_bytes`] oder mit dem Auth-Header.
    pub fn download_url(&self, id: &str) -> Result<String> {
        Ok(self
            .url(&format!("/Items/{}/Download", enc(id)), &[])?
            .into())
    }

    /// Adresse des Profilbilds eines Benutzers (ohne Token).
    pub fn profile_image_url(&self, user_id: &str, tag: Option<&str>) -> Result<String> {
        let mut query = vec![("userId".to_owned(), user_id.to_owned())];
        if let Some(tag) = tag {
            query.push(("tag".to_owned(), tag.to_owned()));
        }
        Ok(self.url("/UserImage", &query)?.into())
    }

    // ------------------------------------------------------------ Musik

    /// Künstler (`/Artists`).
    pub async fn artists(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/Artists", &self.with_user(query.build())?)
            .await
    }

    /// Albumkünstler (`/Artists/AlbumArtists`).
    pub async fn album_artists(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/Artists/AlbumArtists", &self.with_user(query.build())?)
            .await
    }

    /// Alben, optional eines Künstlers und/oder einer Bibliothek.
    pub async fn albums(
        &self,
        artist_id: Option<&str>,
        parent_id: Option<&str>,
        limit: u32,
        start_index: u32,
    ) -> Result<QueryResult<BaseItem>> {
        let mut query = ItemsQuery::new()
            .include_types(&[ItemKind::MusicAlbum])
            .recursive(true)
            .sort_by(&[SortBy::SortName])
            .order(SortOrder::Ascending)
            .limit(limit)
            .start_index(start_index)
            .total_count(true);
        if let Some(a) = artist_id {
            query = query.artist_ids(&[a]);
        }
        if let Some(p) = parent_id {
            query = query.parent_id(p);
        }
        self.items(&query).await
    }

    /// Titel eines Albums in Reihenfolge (Disk, Nummer).
    pub async fn album_tracks(&self, album_id: &str) -> Result<QueryResult<BaseItem>> {
        self.items(
            &ItemsQuery::new()
                .parent_id(album_id)
                .include_types(&[ItemKind::Audio])
                .raw("sortBy", "ParentIndexNumber,IndexNumber,SortName")
                .fields(&["MediaSources"]),
        )
        .await
    }

    /// Titel (alle oder gefiltert, z. B. nach Künstler oder Suchwort).
    pub async fn tracks(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.items(
            &query
                .clone()
                .include_types(&[ItemKind::Audio])
                .recursive(true),
        )
        .await
    }

    /// Zufallsmix (Radio) zu einem Titel, Album, Künstler, Genre oder einer Liste (`/Items/{id}/InstantMix`).
    pub async fn instant_mix(&self, id: &str, limit: u32) -> Result<QueryResult<BaseItem>> {
        self.get_json(
            &format!("/Items/{}/InstantMix", enc(id)),
            &self.with_user(&[("limit".to_owned(), limit.to_string())])?,
        )
        .await
    }

    /// Stream-Adresse eines Titels (`/Audio/{id}/universal`) **ohne Token**. mpv bekommt den Auth-Header über
    /// `http-header-fields` ([`Client::auth_header_value`]).
    pub fn audio_stream_url(&self, item_id: &str, options: &AudioStreamOptions) -> Result<String> {
        let mut query = vec![
            ("userId".to_owned(), self.user_id()?),
            ("deviceId".to_owned(), self.info().device_id.clone()),
            ("container".to_owned(), options.containers.join(",")),
            ("audioCodec".to_owned(), options.audio_codec.clone()),
            (
                "transcodingContainer".to_owned(),
                options.transcoding_container.clone(),
            ),
            (
                "transcodingProtocol".to_owned(),
                options.transcoding_protocol.clone(),
            ),
            (
                "maxStreamingBitrate".to_owned(),
                options.max_streaming_bitrate.to_string(),
            ),
        ];
        if let Some(t) = options.start_time_ticks.filter(|t| *t > 0) {
            query.push(("startTimeTicks".to_owned(), t.to_string()));
        }
        Ok(self
            .url(&format!("/Audio/{}/universal", enc(item_id)), &query)?
            .into())
    }

    // ------------------------------------------------------------ Fotos, Bücher

    /// Inhalt eines Fotoordners (Unterordner/Alben und Fotos), nicht rekursiv; ohne `parent_id` die Wurzel
    /// aller Foto-Bibliotheken.
    pub async fn photo_items(
        &self,
        parent_id: Option<&str>,
        limit: u32,
        start_index: u32,
    ) -> Result<QueryResult<BaseItem>> {
        let mut query = ItemsQuery::new()
            .raw("sortBy", "IsFolder,SortName")
            .fields(&["PrimaryImageAspectRatio"])
            .limit(limit)
            .start_index(start_index)
            .total_count(true);
        match parent_id {
            Some(p) => query = query.parent_id(p),
            None => {
                query = query
                    .recursive(true)
                    .include_types(&[ItemKind::PhotoAlbum, ItemKind::Photo])
            }
        }
        self.items(&query).await
    }

    /// Bücher (und Hörbücher), optional einer Bibliothek.
    pub async fn books(
        &self,
        parent_id: Option<&str>,
        limit: u32,
        start_index: u32,
    ) -> Result<QueryResult<BaseItem>> {
        let mut query = ItemsQuery::new()
            .include_types(&[ItemKind::Book, ItemKind::AudioBook])
            .recursive(true)
            .sort_by(&[SortBy::SortName])
            .limit(limit)
            .start_index(start_index)
            .total_count(true);
        if let Some(p) = parent_id {
            query = query.parent_id(p);
        }
        self.items(&query).await
    }

    // ------------------------------------------------------------ Wiedergabelisten

    /// Legt eine Wiedergabeliste an (`media_type` z. B. `Audio` oder `Video`).
    pub async fn create_playlist(
        &self,
        name: &str,
        item_ids: &[&str],
        media_type: Option<&str>,
    ) -> Result<PlaylistCreationResult> {
        let mut extra = vec![("name".to_owned(), name.to_owned())];
        if !item_ids.is_empty() {
            extra.push(("ids".to_owned(), item_ids.join(",")));
        }
        opt(&mut extra, "mediaType", media_type.map(str::to_owned));
        self.call_json(
            Method::POST,
            "/Playlists",
            &self.with_user(&extra)?,
            NO_BODY,
        )
        .await
    }

    /// Fügt Elemente an (`position` = Einfügestelle, sonst am Ende).
    pub async fn playlist_add(
        &self,
        playlist_id: &str,
        item_ids: &[&str],
        position: Option<u32>,
    ) -> Result<()> {
        let mut extra = vec![("ids".to_owned(), item_ids.join(","))];
        opt(&mut extra, "position", position.map(|p| p.to_string()));
        self.call_empty(
            Method::POST,
            &format!("/Playlists/{}/Items", enc(playlist_id)),
            &self.with_user(&extra)?,
            NO_BODY,
        )
        .await
    }

    /// Entfernt Einträge. Gebraucht werden die `PlaylistItemId`-Werte der Einträge, nicht die Element-Ids.
    pub async fn playlist_remove(&self, playlist_id: &str, entry_ids: &[&str]) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            &format!("/Playlists/{}/Items", enc(playlist_id)),
            &q(&[("entryIds", entry_ids.join(","))]),
            NO_BODY,
        )
        .await
    }

    /// Verschiebt einen Eintrag (`PlaylistItemId`) an die neue Stelle (0-basiert).
    pub async fn playlist_move(
        &self,
        playlist_id: &str,
        playlist_item_id: &str,
        new_index: u32,
    ) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!(
                "/Playlists/{}/Items/{}/Move/{new_index}",
                enc(playlist_id),
                enc(playlist_item_id)
            ),
            &[],
            NO_BODY,
        )
        .await
    }

    /// Einträge einer Wiedergabeliste in Reihenfolge; jedes Element trägt `playlist_item_id`.
    pub async fn playlist_items(
        &self,
        playlist_id: &str,
        limit: u32,
        start_index: u32,
        fields: &[&str],
    ) -> Result<QueryResult<BaseItem>> {
        let mut extra = vec![
            ("limit".to_owned(), limit.to_string()),
            ("startIndex".to_owned(), start_index.to_string()),
        ];
        fields_opt(&mut extra, fields);
        self.get_json(
            &format!("/Playlists/{}/Items", enc(playlist_id)),
            &self.with_user(&extra)?,
        )
        .await
    }

    /// Einstellungen und Element-Ids einer Wiedergabeliste.
    pub async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDto> {
        self.get_json(&format!("/Playlists/{}", enc(playlist_id)), &[])
            .await
    }

    /// Benennt eine Wiedergabeliste um.
    pub async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!("/Playlists/{}", enc(playlist_id)),
            &[],
            Some(&serde_json::json!({ "Name": name })),
        )
        .await
    }
}
