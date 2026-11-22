//! The custom GPUI element that paints the document and registers the text
//! input handler.

use gpui::{
    point, px, size, App, Bounds, Element, ElementId, ElementInputHandler, Entity, GlobalElementId,
    LayoutId, Pixels, Style, TextAlign, Window,
};
use silvestris::ActiveTheme;

use crate::editor::Editor;
use crate::editor_paint::PaintFrame;
use crate::geometry::Geometry;

pub(crate) struct EditorElement {
    pub(crate) editor: Entity<Editor>,
}

pub(crate) struct EditorPrepaint {
    frame: PaintFrame,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style {
            position: gpui::Position::Absolute,
            ..Default::default()
        };
        style.inset.top = px(0.).into();
        style.inset.left = px(0.).into();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> EditorPrepaint {
        let theme = cx.theme().clone();
        let frame = self.editor.update(cx, |editor, _| {
            editor.build_visible_rows(window, bounds, &theme)
        });
        EditorPrepaint { frame }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut EditorPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let clip = Bounds::new(
            bounds.origin + point(px(8.), px(8.)),
            size(
                (bounds.size.width - px(16.)).max(px(0.)),
                (bounds.size.height - px(16.)).max(px(0.)),
            ),
        );
        window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
            for quad in &prepaint.frame.selection {
                window.paint_quad(quad.clone());
            }
            for row in &prepaint.frame.rows {
                row.line
                    .paint(
                        row.origin,
                        prepaint.frame.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }
            if let Some(caret) = prepaint.frame.caret.clone() {
                window.paint_quad(caret);
            }
        });
        self.editor.update(cx, |editor, _| {
            editor.geometry = Some(Geometry {
                bounds,
                line_height: prepaint.frame.line_height,
                char_width: prepaint.frame.char_width,
            });
        });
    }
}
