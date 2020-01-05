//! A Renderer for windows consoles

use crate::caps::Capabilities;
use crate::cell::{AttributeChange, CellAttributes, Underline};
use crate::color::{AnsiColor, ColorAttribute};
use crate::surface::{Change, Position};
use crate::terminal::windows::{ConsoleInputHandle, ConsoleOutputHandle};
use num;
use std::io::{Read, Write};
use winapi::um::wincon::{
    BACKGROUND_BLUE, BACKGROUND_GREEN, BACKGROUND_INTENSITY, BACKGROUND_RED,
    COMMON_LVB_REVERSE_VIDEO, COMMON_LVB_UNDERSCORE, FOREGROUND_BLUE, FOREGROUND_GREEN,
    FOREGROUND_INTENSITY, FOREGROUND_RED,
};

pub struct WindowsConsoleRenderer {
    current_attr: CellAttributes,
}

impl WindowsConsoleRenderer {
    pub fn new(_caps: Capabilities) -> Self {
        Self {
            current_attr: CellAttributes::default(),
        }
    }
}

fn to_attr_word(attr: &CellAttributes) -> u16 {
    macro_rules! ansi_colors_impl {
        ($idx:expr, $default:ident,
                $red:ident, $green:ident, $blue:ident,
                $bright:ident, $( ($variant:ident, $bits:expr) ),*) =>{
            match num::FromPrimitive::from_u8($idx).unwrap_or(AnsiColor::$default) {
                $(
                    AnsiColor::$variant => $bits,
                )*
            }
        }
    }

    macro_rules! ansi_colors {
        ($idx:expr, $default:ident, $red:ident, $green:ident, $blue:ident, $bright:ident) => {
            ansi_colors_impl!(
                $idx,
                $default,
                $red,
                $green,
                $blue,
                $bright,
                (Black, 0),
                (Maroon, $red),
                (Green, $green),
                (Olive, $red | $green),
                (Navy, $blue),
                (Purple, $red | $blue),
                (Teal, $green | $blue),
                (Silver, $red | $green | $blue),
                (Grey, $bright),
                (Red, $bright | $red),
                (Lime, $bright | $green),
                (Yellow, $bright | $red | $green),
                (Blue, $bright | $blue),
                (Fuschia, $bright | $red | $blue),
                (Aqua, $bright | $green | $blue),
                (White, $bright | $red | $green | $blue)
            )
        };
    }

    let fg = match attr.foreground {
        ColorAttribute::TrueColorWithDefaultFallback(_) | ColorAttribute::Default => {
            FOREGROUND_BLUE | FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_INTENSITY
        }

        ColorAttribute::TrueColorWithPaletteFallback(_, idx)
        | ColorAttribute::PaletteIndex(idx) => ansi_colors!(
            idx,
            White,
            FOREGROUND_RED,
            FOREGROUND_GREEN,
            FOREGROUND_BLUE,
            FOREGROUND_INTENSITY
        ),
    };

    let bg = match attr.background {
        ColorAttribute::TrueColorWithDefaultFallback(_) | ColorAttribute::Default => 0,
        ColorAttribute::TrueColorWithPaletteFallback(_, idx)
        | ColorAttribute::PaletteIndex(idx) => ansi_colors!(
            idx,
            Black,
            BACKGROUND_RED,
            BACKGROUND_GREEN,
            BACKGROUND_BLUE,
            BACKGROUND_INTENSITY
        ),
    };

    let reverse = if attr.reverse() {
        COMMON_LVB_REVERSE_VIDEO
    } else {
        0
    };
    let underline = if attr.underline() != Underline::None {
        COMMON_LVB_UNDERSCORE
    } else {
        0
    };

    bg | fg | reverse | underline
}

impl WindowsConsoleRenderer {
    pub fn render_to<A: ConsoleInputHandle + Read, B: ConsoleOutputHandle + Write>(
        &mut self,
        changes: &[Change],
        _read: &mut A,
        out: &mut B,
    ) -> anyhow::Result<()> {
        for change in changes {
            match change {
                Change::ClearScreen(color) => {
                    out.flush()?;
                    self.current_attr = CellAttributes::default()
                        .set_background(color.clone())
                        .clone();

                    let info = out.get_buffer_info()?;
                    // We want to clear only the viewport; we don't want to toss out
                    // the scrollback.
                    if info.srWindow.Left != 0 {
                        // The user has scrolled the viewport horizontally; let's move
                        // it back to the left for the sake of sanity
                        out.set_viewport(
                            0,
                            info.srWindow.Top,
                            info.srWindow.Right - info.srWindow.Left,
                            info.srWindow.Bottom,
                        )?;
                    }
                    // Clear the full width of the buffer (not the viewport size)
                    let visible_width = info.dwSize.X as u32;
                    // And clear all of the visible lines from this point down
                    let visible_height = info.dwSize.Y as u32 - info.srWindow.Top as u32;
                    let num_spaces = visible_width * visible_height;
                    out.fill_char(' ', 0, info.srWindow.Top, num_spaces as u32)?;
                    out.fill_attr(
                        to_attr_word(&self.current_attr),
                        0,
                        info.srWindow.Top,
                        num_spaces as u32,
                    )?;
                    out.set_cursor_position(0, info.srWindow.Top)?;
                }
                Change::ClearToEndOfLine(color) => {
                    out.flush()?;
                    self.current_attr = CellAttributes::default()
                        .set_background(color.clone())
                        .clone();

                    let info = out.get_buffer_info()?;
                    let width =
                        (info.dwSize.X as u32).saturating_sub(info.dwCursorPosition.X as u32);
                    out.fill_char(' ', info.dwCursorPosition.X, info.dwCursorPosition.Y, width)?;
                    out.fill_attr(
                        to_attr_word(&self.current_attr),
                        info.dwCursorPosition.X,
                        info.dwCursorPosition.Y,
                        width,
                    )?;
                }
                Change::ClearToEndOfScreen(color) => {
                    out.flush()?;
                    self.current_attr = CellAttributes::default()
                        .set_background(color.clone())
                        .clone();

                    let info = out.get_buffer_info()?;
                    let width =
                        (info.dwSize.X as u32).saturating_sub(info.dwCursorPosition.X as u32);
                    out.fill_char(' ', info.dwCursorPosition.X, info.dwCursorPosition.Y, width)?;
                    out.fill_attr(
                        to_attr_word(&self.current_attr),
                        info.dwCursorPosition.X,
                        info.dwCursorPosition.Y,
                        width,
                    )?;
                    // Clear the full width of the buffer (not the viewport size)
                    let visible_width = info.dwSize.X as u32;
                    // And clear all of the visible lines below the cursor
                    let visible_height =
                        (info.dwSize.Y as u32).saturating_sub((info.dwCursorPosition.Y as u32) + 1);
                    let num_spaces = visible_width * visible_height;
                    out.fill_char(' ', 0, info.dwCursorPosition.Y + 1, num_spaces as u32)?;
                    out.fill_attr(
                        to_attr_word(&self.current_attr),
                        0,
                        info.dwCursorPosition.Y + 1,
                        num_spaces as u32,
                    )?;
                }
                Change::Text(text) => {
                    out.flush()?;
                    out.set_attr(to_attr_word(&self.current_attr))?;
                    out.write_all(text.as_bytes())?;
                }
                Change::CursorPosition { x, y } => {
                    out.flush()?;
                    let info = out.get_buffer_info()?;
                    // For horizontal cursor movement, we consider the full width
                    // of the screen buffer, even if the viewport is smaller
                    let x = match x {
                        Position::NoChange => info.dwCursorPosition.X,
                        Position::Absolute(x) => *x as i16,
                        Position::Relative(delta) => info.dwCursorPosition.X + *delta as i16,
                        Position::EndRelative(delta) => info.dwSize.X - *delta as i16,
                    };
                    // For vertical cursor movement, we constrain the movement to
                    // the viewport.
                    let y = match y {
                        Position::NoChange => info.dwCursorPosition.Y,
                        Position::Absolute(y) => info.srWindow.Top + *y as i16,
                        Position::Relative(delta) => info.dwCursorPosition.Y + *delta as i16,
                        Position::EndRelative(delta) => info.srWindow.Bottom - *delta as i16,
                    };

                    out.set_cursor_position(x, y)?;
                }
                Change::Attribute(AttributeChange::Intensity(value)) => {
                    self.current_attr.set_intensity(*value);
                }
                Change::Attribute(AttributeChange::Italic(value)) => {
                    self.current_attr.set_italic(*value);
                }
                Change::Attribute(AttributeChange::Reverse(value)) => {
                    self.current_attr.set_reverse(*value);
                }
                Change::Attribute(AttributeChange::StrikeThrough(value)) => {
                    self.current_attr.set_strikethrough(*value);
                }
                Change::Attribute(AttributeChange::Blink(value)) => {
                    self.current_attr.set_blink(*value);
                }
                Change::Attribute(AttributeChange::Invisible(value)) => {
                    self.current_attr.set_invisible(*value);
                }
                Change::Attribute(AttributeChange::Underline(value)) => {
                    self.current_attr.set_underline(*value);
                }
                Change::Attribute(AttributeChange::Foreground(col)) => {
                    self.current_attr.set_foreground(*col);
                }
                Change::Attribute(AttributeChange::Background(col)) => {
                    self.current_attr.set_background(*col);
                }
                Change::Attribute(AttributeChange::Hyperlink(link)) => {
                    self.current_attr.hyperlink = link.clone();
                }
                Change::Attribute(AttributeChange::LineDrawing(value)) => {
                    self.current_attr.set_line_drawing(*value);
                }
                Change::AllAttributes(all) => {
                    self.current_attr = all.clone();
                }
                Change::CursorColor(_color) => {}
                Change::CursorShape(_shape) => {}
                Change::Image(image) => {
                    // Images are not supported, so just blank out the cells and
                    // move the cursor to the right spot
                    out.flush()?;
                    let info = out.get_buffer_info()?;
                    for y in 0..image.height {
                        out.fill_char(
                            ' ',
                            info.dwCursorPosition.X,
                            y as i16 + info.dwCursorPosition.Y,
                            image.width as u32,
                        )?;
                    }
                    out.set_cursor_position(
                        info.dwCursorPosition.X + image.width as i16,
                        info.dwCursorPosition.Y,
                    )?;
                }
                Change::ScrollRegionUp {
                    first_row,
                    region_size,
                    scroll_count,
                } => {
                    if *region_size > 0 {
                        let info = out.get_buffer_info()?;
                        out.scroll_region(
                            info.srWindow.Left,
                            info.srWindow.Top + *first_row as i16,
                            info.srWindow.Right,
                            info.srWindow.Top + *first_row as i16 + *region_size as i16,
                            0,
                            -(*scroll_count as i16),
                            to_attr_word(&self.current_attr),
                        )?;
                    }
                }
                Change::ScrollRegionDown {
                    first_row,
                    region_size,
                    scroll_count,
                } => {
                    if *region_size > 0 {
                        let info = out.get_buffer_info()?;
                        out.scroll_region(
                            info.srWindow.Left,
                            info.srWindow.Top + *first_row as i16,
                            info.srWindow.Right,
                            info.srWindow.Top + *first_row as i16 + *region_size as i16,
                            0,
                            *scroll_count as i16,
                            to_attr_word(&self.current_attr),
                        )?;
                    }
                }
                Change::Title(_text) => {
                    // Don't actually render this for now.
                    // The primary purpose of Change::Title at the time of
                    // writing is to transfer tab titles across domains
                    // in the wezterm multiplexer model.  It's not clear
                    // that it would be a good idea to unilaterally output
                    // eg: a title change escape sequence here in the
                    // renderer because we might be composing multiple widgets
                    // together, each with its own title.
                }
            }
        }
        out.flush()?;
        out.set_attr(to_attr_word(&self.current_attr))?;
        Ok(())
    }
}
