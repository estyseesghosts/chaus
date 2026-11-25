//! Map rc-file theme names onto Silvestris theme ids.

use chaus_core::settings::ThemeName;
use silvestris::ThemeId;

#[must_use]
pub fn theme_id(name: ThemeName) -> ThemeId {
    match name {
        ThemeName::GeistDark => ThemeId::GEIST_DARK,
        ThemeName::GeistLight => ThemeId::GEIST_LIGHT,
        ThemeName::AquaLight => ThemeId::AQUA_LIGHT,
        ThemeName::AquaDark => ThemeId::AQUA_DARK,
        _ => ThemeId::GEIST_DARK,
    }
}
