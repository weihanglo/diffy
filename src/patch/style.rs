use anstyle::AnsiColor;
use anstyle::Style;

pub const DELETE: Style = AnsiColor::Red.on_default();
pub const HUNK_HEADER: Style = AnsiColor::Cyan.on_default();
pub const INSERT: Style = AnsiColor::Green.on_default();
pub const PATCH_HEADER: Style = Style::new().bold();
