//! OWNER: account-wardrobe agent. Rust side of AccountVault / Wardrobe /
//! AccountPopover (ui/pages/account-wardrobe).
//!
//! * [`vault`]: accounts (shared by the vault page and the TopChrome popover),
//!   offline forge, Microsoft device flow, avatars, client id, Skin Atelier.
//! * [`wardrobe`]: identity verification, Discord link, cosmetics (#/armario).

use slint::ComponentHandle;

use crate::bridge::Ctx;
use crate::{AccountBridge, AppWindow};

mod image;
mod vault;
mod wardrobe;

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<AccountBridge>();
    b.on_valid_offline_name(|name| valid_offline_name(name.trim()));
    b.on_truncate(|text, n| text.chars().take(n.max(0) as usize).collect::<String>().into());
    b.on_trim(|text| text.trim().into());
    vault::init(ctx, ui);
    wardrobe::init(ctx, ui);
}

/// `^[A-Za-z0-9_]{2,16}$` (AccountVault nameValid / AccountPopover isValidName).
pub(crate) fn valid_offline_name(name: &str) -> bool {
    (2..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `^[A-Za-z0-9_]{3,16}$` (Wardrobe MC_NAME_RE, hidden players).
pub(crate) fn valid_mc_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The message the Svelte UI showed for a failed call (`e.message`).
pub(crate) fn err_msg(e: &espectral_engine::EngineError) -> String {
    e.envelope().message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rules() {
        assert!(valid_offline_name("Steve"));
        assert!(valid_offline_name("ab"));
        assert!(!valid_offline_name("a"));
        assert!(!valid_offline_name("bad name"));
        assert!(!valid_offline_name("seventeen_chars_x"));
        assert!(valid_mc_name("Notch"));
        assert!(!valid_mc_name("ab"));
        assert!(!valid_mc_name("ñandu"));
    }
}
