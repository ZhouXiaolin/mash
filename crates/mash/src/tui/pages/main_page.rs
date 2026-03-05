use iocraft::prelude::*;

use crate::tui::components::input_section::InputSection;
use crate::tui::components::status_line::StatusLine;

/// Main page: status line + input section, pushed to the bottom.
#[component]
pub fn MainPage(hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let _ = hooks;

    element! {
        View(
            flex_direction: FlexDirection::Column,
            height: 100pct,
            width: 100pct,
            padding: 1,
            justify_content: JustifyContent::End,
        ) {
            StatusLine
            InputSection
        }
    }
}
