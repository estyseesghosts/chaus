//! Geist Sans, bundled by chaus and applied through Silvestris font overrides.
//!
//! Silvestris ships Geist Mono but not Geist Sans, and deliberately does not
//! impose a house UI font on its consumers. chaus registers these faces and
//! selects `Geist Sans` for the two Geist themes only; Aqua keeps its measured
//! Lato/Source Code Pro typography.

use std::borrow::Cow;

use gpui::App;

/// Family name resolved from the bundled Geist Sans faces. Vercel's static
/// files name the family `Geist`; the product name for the same face is Geist
/// Sans.
pub const GEIST_SANS: &str = "Geist";

const FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/geist-sans/Geist-Regular.ttf"),
    include_bytes!("../assets/fonts/geist-sans/Geist-Italic.ttf"),
    include_bytes!("../assets/fonts/geist-sans/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/geist-sans/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/geist-sans/Geist-Bold.ttf"),
];

/// Register the bundled faces and prove the family resolved.
pub fn register(cx: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let faces: Vec<Cow<'static, [u8]>> = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    cx.text_system().add_fonts(faces)?;
    if !cx
        .text_system()
        .all_font_names()
        .iter()
        .any(|name| name == GEIST_SANS)
    {
        return Err(format!(
            "bundled Geist Sans faces did not register as {GEIST_SANS}; using the theme's default UI font"
        )
        .into());
    }
    Ok(())
}
