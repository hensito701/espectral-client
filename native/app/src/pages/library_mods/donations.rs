//! DonationsPavilion (`#/donaciones`): static channels + rank catalog
//! (category tabs, search over the translated texts, grid rows) + unbans.

use std::cell::Cell;
use std::time::Duration;

use slint::{Color, ComponentHandle, Timer, TimerMode};

use super::{model, ss};
use crate::bridge::Ctx;
use crate::{i18n, toast, util, AppWindow, LibraryBridge, RankCard, RankGridRow};

struct Rank {
    id: &'static str,
    name: &'static str,
    price: &'static str,
    tagline: &'static str,
    permanent: bool,
    tone: u32,
    badge: Option<&'static str>,
    featured: bool,
    accent: u32,
    perks: &'static [&'static str],
}

const DIAMOND: u32 = 0x38bdf8;
const EMERALD: u32 = 0x10b981;
const GOLD: u32 = 0xf59e0b;
const PURPLE: u32 = 0xa855f7;
const AMBER: u32 = 0xf97316;

const RANKS: [Rank; 7] = [
    Rank {
        id: "vip",
        name: "donations.rank.vip",
        price: "donations.rank.vip.price",
        tagline: "pav.rank.vip.tagline",
        permanent: true,
        tone: DIAMOND,
        badge: None,
        featured: false,
        accent: 0x38bdf8,
        perks: &[
            "donations.rank.vip.perk1",
            "donations.rank.vip.perk2",
            "donations.rank.vip.perk3",
            "donations.rank.vip.perk4",
            "donations.rank.vip.perk5",
            "donations.rank.vip.perk6",
        ],
    },
    Rank {
        id: "vip-plus",
        name: "donations.rank.vipPlus",
        price: "donations.rank.vipPlus.price",
        tagline: "pav.rank.vipPlus.tagline",
        permanent: true,
        tone: EMERALD,
        badge: None,
        featured: false,
        accent: 0x10b981,
        perks: &[
            "donations.rank.vipPlus.perk1",
            "donations.rank.vipPlus.perk2",
            "donations.rank.vipPlus.perk3",
            "donations.rank.vipPlus.perk4",
        ],
    },
    Rank {
        id: "mvp",
        name: "donations.rank.mvp",
        price: "donations.rank.mvp.price",
        tagline: "pav.rank.mvp.tagline",
        permanent: true,
        tone: DIAMOND,
        badge: None,
        featured: false,
        accent: 0x06b6d4,
        perks: &[
            "donations.rank.mvp.perk1",
            "donations.rank.mvp.perk2",
            "donations.rank.mvp.perk3",
            "donations.rank.mvp.perk4",
            "donations.rank.mvp.perk5",
            "donations.rank.mvp.perk6",
            "donations.rank.mvp.perk7",
        ],
    },
    Rank {
        id: "mvp-plus",
        name: "donations.rank.mvpPlus",
        price: "donations.rank.mvpPlus.price",
        tagline: "pav.rank.mvpPlus.tagline",
        permanent: true,
        tone: EMERALD,
        badge: Some("pav.rankFeatured"),
        featured: true,
        accent: 0x059669,
        perks: &["donations.rank.mvpPlus.perk1", "donations.rank.mvpPlus.perk2", "donations.rank.mvpPlus.perk3"],
    },
    Rank {
        id: "custom",
        name: "donations.rank.custom",
        price: "donations.rank.custom.price",
        tagline: "pav.rank.custom.tagline",
        permanent: true,
        tone: GOLD,
        badge: Some("pav.rankExclusive"),
        featured: false,
        accent: 0xf59e0b,
        perks: &["donations.rank.custom.perk1", "donations.rank.custom.perk2"],
    },
    Rank {
        id: "dhost",
        name: "donations.rank.dhost",
        price: "donations.rank.dhost.price",
        tagline: "pav.rank.dhost.tagline",
        permanent: false,
        tone: PURPLE,
        badge: Some("pav.rankMonthly"),
        featured: false,
        accent: 0xa855f7,
        perks: &["donations.rank.dhost.perk1", "donations.rank.dhost.perk2"],
    },
    Rank {
        id: "private-uhc",
        name: "donations.rank.privateUhc",
        price: "donations.rank.privateUhc.price",
        tagline: "pav.rank.privateUhc.tagline",
        permanent: false,
        tone: AMBER,
        badge: Some("pav.rankEvent"),
        featured: false,
        accent: 0xf97316,
        perks: &["donations.rank.privateUhc.perk1", "donations.rank.privateUhc.perk2"],
    },
];

thread_local! {
    static COLS: Cell<usize> = const { Cell::new(3) };
    static COPIED_RESET: Timer = Timer::default();
}

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    super::set_ctx(ctx);
    let b = ui.global::<LibraryBridge>();
    b.set_d_count_all(RANKS.len() as i32);
    b.set_d_count_permanent(RANKS.iter().filter(|r| r.permanent).count() as i32);
    b.set_d_count_services(RANKS.iter().filter(|r| !r.permanent).count() as i32);
    b.on_d_refresh(|| {
        if let Some(ui) = super::ui_handle() {
            refresh(&ui);
        }
    });
    b.on_d_set_cols(|cols| {
        COLS.with(|c| c.set(cols.max(1) as usize));
        if let Some(ui) = super::ui_handle() {
            refresh(&ui);
        }
    });
    b.on_d_copy(|id, url| {
        util::copy_text(&url);
        toast::ok(i18n::t("pav.copiedToast"));
        let Some(ui) = super::ui_handle() else { return };
        ui.global::<LibraryBridge>().set_d_copied(id);
        // useCopy(1600): the "Copiado" state reverts after 1.6 s
        COPIED_RESET.with(|t| {
            t.start(TimerMode::SingleShot, Duration::from_millis(1600), || {
                if let Some(ui) = super::ui_handle() {
                    ui.global::<LibraryBridge>().set_d_copied(ss(""));
                }
            })
        });
    });
    refresh(ui);
}

fn matches(r: &Rank, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let has = |key: &str| i18n::t(key).to_lowercase().contains(q);
    has(r.name) || has(r.price) || has(r.tagline) || r.perks.iter().any(|p| has(p))
}

pub fn refresh(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let category = b.get_d_category();
    let q = b.get_d_query().trim().to_lowercase();
    let cards: Vec<RankCard> = RANKS
        .iter()
        .filter(|r| match category {
            1 => r.permanent,
            2 => !r.permanent,
            _ => true,
        })
        .filter(|r| matches(r, &q))
        .map(|r| RankCard {
            id: ss(r.id),
            name_key: ss(r.name),
            price_key: ss(r.price),
            tagline_key: ss(r.tagline),
            badge_kind: ss(if r.featured {
                "featured"
            } else if r.badge.is_some() {
                "custom"
            } else if r.tone == DIAMOND {
                "diamond"
            } else if r.tone == GOLD {
                "gold"
            } else {
                "emerald"
            }),
            badge_key: ss(r.badge.unwrap_or("pav.rankFeatured")),
            featured: r.featured,
            tone: rgb(r.tone),
            accent: rgb(r.accent),
            perks: model(r.perks.iter().map(ss).collect()),
            wardrobe: r.id == "mvp" || r.id == "mvp-plus",
        })
        .collect();
    let cols = COLS.with(|c| c.get());
    let rows: Vec<RankGridRow> = cards.chunks(cols).map(|c| RankGridRow { cards: model(c.to_vec()) }).collect();
    b.set_d_rows(model(rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        assert_eq!(RANKS.iter().filter(|r| r.permanent).count(), 5);
        assert_eq!(RANKS.iter().filter(|r| !r.permanent).count(), 2);
        assert!(matches(&RANKS[0], ""));
    }
}
