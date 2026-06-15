pub(crate) mod folder_dialog;
pub(crate) mod gif_animation;
pub(crate) mod prompt_popup;
pub(crate) mod settings_panel;
pub(crate) mod slint_views;
pub(crate) mod theme;
pub(crate) mod window_icon;
pub(crate) mod window_position;

mod window;

pub(crate) use gif_animation::init_animation_store;
pub(crate) use window::run_window;

/// Register the bundled Maple Mono CN font into Slint's shared font
/// collection so every Slint window renders Latin + CJK (incl. fullwidth
/// punctuation like 【】。) from one monospace family, with no dependency on
/// system fonts or the femtovg renderer's glyph fallback. Regular + Bold are
/// bundled, each subset to GB2312 (everyday Simplified Chinese) to keep them
/// ~5 MB apiece; a hanzi outside that set would fall back / render as tofu.
///
/// Slint 1.16's `shared_collection()` panics until the platform is
/// initialized, so this must run inside the event loop. Call it before
/// building any Slint window; the `Once` makes repeat calls cheap no-ops.
pub(crate) fn ensure_embedded_fonts() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use slint::fontique_08::fontique;
        static REGULAR: &[u8] = include_bytes!("../../assets/fonts/MapleMonoCN-Regular.ttf");
        static BOLD: &[u8] = include_bytes!("../../assets/fonts/MapleMonoCN-Bold.ttf");
        let mut collection = slint::fontique_08::shared_collection();
        for font in [REGULAR, BOLD] {
            let blob = fontique::Blob::new(std::sync::Arc::new(font));
            collection.register_fonts(blob, None);
        }
    });
}
