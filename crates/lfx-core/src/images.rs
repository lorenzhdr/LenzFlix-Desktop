//! Bildwahl und Bildgrößen (Port von `pickLandscape`, `logoImage`, `imageUrl` aus `core/api.js` und der
//! Kartenbilder aus `rows/card.js`). Es entstehen [`ImageRef`]s (Item, Art, Tag, Breite), keine URLs mit Token;
//! die URL baut `lfx_api::Client::image_url`.

use crate::datasaver::scaled;
use crate::util::non_empty;
use lfx_api::{BaseItem, Client, ImageOptions, ImageType, ItemKind};

/// Gewähltes Bild: Besitzer-Id, Art, Tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePick {
    /// Item-Id, zu der das Bild gehört (kann Eltern/Serie sein).
    pub id: String,
    /// Bildart.
    pub kind: ImageType,
    /// Cache-Tag.
    pub tag: String,
}

/// Bild mit Größe (nach Skala), bereit für die URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Bild.
    pub pick: ImagePick,
    /// Höchstbreite in Pixeln.
    pub max_width: u32,
    /// JPEG-Qualität.
    pub quality: u8,
}

impl ImageRef {
    /// URL ohne Token (`/Items/{id}/Images/{art}?maxWidth&quality&tag`).
    pub fn url(&self, client: &Client) -> Result<String, lfx_api::ApiError> {
        client.image_url(
            &self.pick.id,
            self.pick.kind,
            Some(&self.pick.tag),
            ImageOptions {
                max_width: Some(self.max_width),
                max_height: None,
                quality: Some(self.quality),
            },
        )
    }
}

/// Eigenes Bild (`imageUrl`): `Backdrop` nimmt das erste Backdrop-Tag, sonst `ImageTags[art]`.
pub fn own_image(item: &BaseItem, kind: ImageType) -> Option<ImagePick> {
    let tag = if kind == ImageType::Backdrop {
        item.backdrop_image_tags.first().map(String::as_str)
    } else {
        item.image_tags
            .get(kind.as_str())
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }
    .filter(|t| !t.is_empty())?;
    (!item.id.is_empty()).then(|| ImagePick {
        id: item.id.clone(),
        kind,
        tag: tag.to_owned(),
    })
}

/// Bestes Querformat: Thumb, Backdrop, Eltern-Thumb, Eltern-Backdrop, Primary, Serien-Primary.
pub fn pick_landscape(item: &BaseItem) -> Option<ImagePick> {
    let candidates: [(Option<&str>, ImageType, Option<&str>); 6] = [
        (
            Some(&item.id),
            ImageType::Thumb,
            item.image_tags.get("Thumb").map(String::as_str),
        ),
        (
            Some(&item.id),
            ImageType::Backdrop,
            item.backdrop_image_tags.first().map(String::as_str),
        ),
        (
            non_empty(&item.parent_thumb_item_id),
            ImageType::Thumb,
            non_empty(&item.parent_thumb_image_tag),
        ),
        (
            non_empty(&item.parent_backdrop_item_id),
            ImageType::Backdrop,
            item.parent_backdrop_image_tags.first().map(String::as_str),
        ),
        (
            Some(&item.id),
            ImageType::Primary,
            item.image_tags.get("Primary").map(String::as_str),
        ),
        (
            non_empty(&item.series_id),
            ImageType::Primary,
            non_empty(&item.series_primary_image_tag),
        ),
    ];
    for (id, kind, tag) in candidates {
        if let (Some(id), Some(tag)) = (id, tag)
            && !id.is_empty()
            && !tag.is_empty()
        {
            return Some(ImagePick {
                id: id.to_owned(),
                kind,
                tag: tag.to_owned(),
            });
        }
    }
    None
}

/// Titel-Logo: eigenes, sonst das der Eltern (`logoImage`).
pub fn pick_logo(item: &BaseItem) -> Option<ImagePick> {
    if let Some(tag) = item
        .image_tags
        .get("Logo")
        .filter(|t| !t.is_empty())
        .filter(|_| !item.id.is_empty())
    {
        return Some(ImagePick {
            id: item.id.clone(),
            kind: ImageType::Logo,
            tag: tag.clone(),
        });
    }
    match (
        non_empty(&item.parent_logo_item_id),
        non_empty(&item.parent_logo_image_tag),
    ) {
        (Some(id), Some(tag)) => Some(ImagePick {
            id: id.to_owned(),
            kind: ImageType::Logo,
            tag: tag.to_owned(),
        }),
        _ => None,
    }
}

/// Querformat einer Karte (`landscapeFor`): Folgen bevorzugen ihr eigenes Thumb/Primary.
/// Rückgabe: Bild und ob es ein Thumb ist (dann trägt es den Titel selbst).
pub fn landscape_for(item: &BaseItem) -> Option<(ImagePick, bool)> {
    if item.kind == ItemKind::Episode {
        for kind in [ImageType::Thumb, ImageType::Primary] {
            if let Some(p) = own_image(item, kind) {
                return Some((p, false));
            }
        }
    }
    pick_landscape(item).map(|p| {
        let thumb = p.kind == ImageType::Thumb;
        (p, thumb)
    })
}

/// Hochformat-Poster (`posterUrl`): Folgen nutzen das Serien-Primary.
pub fn pick_poster(item: &BaseItem) -> Option<ImagePick> {
    if item.kind == ItemKind::Episode {
        return match (
            non_empty(&item.series_id),
            non_empty(&item.series_primary_image_tag),
        ) {
            (Some(id), Some(tag)) => Some(ImagePick {
                id: id.to_owned(),
                kind: ImageType::Primary,
                tag: tag.to_owned(),
            }),
            _ => None,
        };
    }
    own_image(item, ImageType::Primary)
}

/// Bildplätze mit den Web-Breiten (CSS-Pixel bei etwa doppelter Pixeldichte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Querformat-Karte (Handy 420, sonst 600), Qualität 85.
    CardLandscape,
    /// Hochformat-Poster / Top-10-Bild (300), Qualität 85.
    CardPoster,
    /// Logo auf Karten und „Neu und beliebt“ (400), Qualität 90.
    CardLogo,
    /// Billboard-Hintergrund (1600), Qualität 80.
    BillboardBackdrop,
    /// Billboard-Poster am Handy (720), Qualität 80.
    BillboardPoster,
    /// Billboard-Logo (720), Qualität 90.
    BillboardLogo,
    /// Farbprobe für Ambient (64), Qualität 60.
    Ambient,
    /// „Neu und beliebt“-Hintergrund (1000), Qualität 85.
    NewPopularBackdrop,
}

impl Slot {
    /// Web-Basisbreite in Pixeln.
    pub fn base_width(self, phone: bool) -> u32 {
        match self {
            Slot::CardLandscape => {
                if phone {
                    420
                } else {
                    600
                }
            }
            Slot::CardPoster => 300,
            Slot::CardLogo => 400,
            Slot::BillboardBackdrop => 1600,
            Slot::BillboardPoster | Slot::BillboardLogo => 720,
            Slot::Ambient => 64,
            Slot::NewPopularBackdrop => 1000,
        }
    }

    /// Qualität wie im Web.
    pub fn quality(self) -> u8 {
        match self {
            Slot::CardLandscape | Slot::CardPoster | Slot::NewPopularBackdrop => 85,
            Slot::BillboardBackdrop | Slot::BillboardPoster => 80,
            Slot::CardLogo | Slot::BillboardLogo => 90,
            Slot::Ambient => 60,
        }
    }
}

/// Gewünschte Pixelbreite: Web-Basisbreite × Datensparskala × Pixeldichte-Faktor.
///
/// Der Faktor `dpr / 2` (begrenzt auf 0,5–1,5) ist eine **Desktop-Erweiterung**: Die Web-Breiten sind für
/// etwa doppelte Pixeldichte gewählt, bei `dpr = 2` ergibt sich also exakt der Web-Wert (`dpr = 1` halbiert,
/// aber mit Untergrenze 0,5). Die Skala wirkt wie `setImageScale`, die Pixeldichte danach.
pub fn pixel_width(slot: Slot, phone: bool, dpr: f64, scale: f64) -> u32 {
    let base = scaled(slot.base_width(phone), scale);
    let factor = if dpr.is_finite() && dpr > 0.0 {
        (dpr / 2.0).clamp(0.5, 1.5)
    } else {
        1.0
    };
    if factor == 1.0 {
        base
    } else {
        ((f64::from(base) * factor).round().max(1.0)) as u32
    }
}

/// Bildwahl einer Karte (`createCard`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardImages {
    /// Hauptbild (Querformat bzw. Top-10-Poster).
    pub main: Option<ImageRef>,
    /// Poster für den Handy-`source` (nur Querformat-Zeilen).
    pub poster: Option<ImageRef>,
    /// Logo, wenn das Bild den Titel nicht selbst zeigt.
    pub logo: Option<ImageRef>,
    /// Titel als Text statt Logo (Bild ohne Thumb und kein Logo).
    pub title_text: bool,
    /// Querformat-Bild ohne Titel (`lfx-card__link--titled`).
    pub titled: bool,
}

/// Kartenart der Zeile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// Querformat (mit Poster-Rückfall).
    Landscape,
    /// Weiterschauen (mit Fortschrittsbalken).
    Resume,
    /// Top 10 (Poster mit Rangzahl).
    Top10,
}

fn with(pick: ImagePick, slot: Slot, phone: bool, dpr: f64, scale: f64) -> ImageRef {
    ImageRef {
        pick,
        max_width: pixel_width(slot, phone, dpr, scale),
        quality: slot.quality(),
    }
}

/// Wählt die Bilder einer Karte wie `createCard`.
pub fn card_images(
    item: &BaseItem,
    kind: CardKind,
    phone: bool,
    dpr: f64,
    scale: f64,
) -> CardImages {
    if kind == CardKind::Top10 {
        let main = own_image(item, ImageType::Primary)
            .or_else(|| pick_landscape(item))
            .map(|p| with(p, Slot::CardPoster, phone, dpr, scale));
        return CardImages {
            title_text: main.is_none(),
            main,
            poster: None,
            logo: None,
            titled: false,
        };
    }
    let land = landscape_for(item);
    let poster = (kind == CardKind::Landscape)
        .then(|| pick_poster(item))
        .flatten();
    let main = match (&land, &poster) {
        (Some((p, _)), _) => Some(with(p.clone(), Slot::CardLandscape, phone, dpr, scale)),
        (None, Some(p)) => Some(with(p.clone(), Slot::CardPoster, phone, dpr, scale)),
        _ => None,
    };
    let titled = land.as_ref().is_none_or(|(_, thumb)| !thumb);
    let logo = if titled {
        pick_logo(item).map(|p| with(p, Slot::CardLogo, phone, dpr, scale))
    } else {
        None
    };
    CardImages {
        main,
        poster: poster.map(|p| with(p, Slot::CardPoster, phone, dpr, scale)),
        title_text: titled && logo.is_none(),
        logo,
        titled,
    }
}

/// Anzeigetitel einer Karte: Folgen/Staffeln zeigen den Serientitel (`displayTitle`).
pub fn display_title(item: &BaseItem) -> String {
    if matches!(item.kind, ItemKind::Episode | ItemKind::Season)
        && let Some(s) = non_empty(&item.series_name)
    {
        return s.to_owned();
    }
    item.name.clone()
}

/// Beschriftung für Bedienhilfen (`ariaLabel`): „Serie – Folge“.
pub fn aria_label(item: &BaseItem) -> String {
    if matches!(item.kind, ItemKind::Episode | ItemKind::Season)
        && let Some(s) = non_empty(&item.series_name)
    {
        return format!("{s} – {}", item.name).trim().to_owned();
    }
    item.name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie() -> BaseItem {
        let mut m = BaseItem {
            id: "m1".into(),
            kind: ItemKind::Movie,
            name: "Film".into(),
            ..Default::default()
        };
        m.image_tags.insert("Primary".into(), "p".into());
        m.image_tags.insert("Logo".into(), "l".into());
        m.backdrop_image_tags = vec!["b".into()];
        m
    }

    #[test]
    fn landscape_order_matches_web() {
        let mut m = movie();
        assert_eq!(pick_landscape(&m).unwrap().kind, ImageType::Backdrop);
        m.image_tags.insert("Thumb".into(), "t".into());
        assert_eq!(pick_landscape(&m).unwrap().kind, ImageType::Thumb);
        m.image_tags.clear();
        m.backdrop_image_tags.clear();
        m.parent_backdrop_item_id = Some("par".into());
        m.parent_backdrop_image_tags = vec!["pb".into()];
        let p = pick_landscape(&m).unwrap();
        assert_eq!((p.id.as_str(), p.kind), ("par", ImageType::Backdrop));
        assert!(pick_landscape(&BaseItem::default()).is_none());
    }

    #[test]
    fn logo_prefers_own_then_parent() {
        let mut m = movie();
        assert_eq!(pick_logo(&m).unwrap().id, "m1");
        m.image_tags.remove("Logo");
        assert!(pick_logo(&m).is_none());
        m.parent_logo_item_id = Some("s".into());
        m.parent_logo_image_tag = Some("x".into());
        assert_eq!(pick_logo(&m).unwrap().id, "s");
    }

    #[test]
    fn episode_uses_series_poster() {
        let e = BaseItem {
            id: "e".into(),
            kind: ItemKind::Episode,
            series_id: Some("s".into()),
            series_primary_image_tag: Some("sp".into()),
            series_name: Some("Serie".into()),
            name: "Folge 1".into(),
            ..Default::default()
        };
        assert_eq!(pick_poster(&e).unwrap().id, "s");
        assert_eq!(display_title(&e), "Serie");
        assert_eq!(aria_label(&e), "Serie – Folge 1");
    }

    #[test]
    fn widths_follow_web_at_dpr2() {
        assert_eq!(pixel_width(Slot::CardLandscape, false, 2.0, 1.0), 600);
        assert_eq!(pixel_width(Slot::CardLandscape, true, 2.0, 1.0), 420);
        assert_eq!(pixel_width(Slot::CardLandscape, false, 2.0, 2.0 / 3.0), 400);
        assert_eq!(pixel_width(Slot::CardLandscape, false, 1.0, 1.0), 300);
        assert_eq!(pixel_width(Slot::BillboardBackdrop, false, 3.0, 1.0), 2400);
    }

    #[test]
    fn card_images_kinds() {
        let m = movie();
        let c = card_images(&m, CardKind::Landscape, false, 2.0, 1.0);
        assert_eq!(c.main.as_ref().unwrap().max_width, 600);
        assert!(c.poster.is_some() && c.logo.is_some() && c.titled);
        let r = card_images(&m, CardKind::Resume, false, 2.0, 1.0);
        assert!(r.poster.is_none());
        let t = card_images(&m, CardKind::Top10, false, 2.0, 1.0);
        let main = t.main.unwrap();
        assert_eq!((main.pick.kind, main.max_width), (ImageType::Primary, 300));
        let bare = card_images(&BaseItem::default(), CardKind::Landscape, false, 2.0, 1.0);
        assert!(bare.main.is_none() && bare.title_text);
    }
}
