//! Three-row, 24-slot display bank plus explicit Master. No authority or device state.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Off,
    On,
    Unknown,
    Preparing,
    Finalizing,
    Fault,
}
impl Flag {
    fn symbol(self, on: char) -> char {
        match self {
            Self::Off => ' ',
            Self::On => on,
            Self::Unknown => '?',
            Self::Preparing => 'P',
            Self::Finalizing => 'F',
            Self::Fault => '!',
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub number: usize,
    pub available: bool,
    pub selected: bool,
    pub recording: Flag,
    pub solo: Flag,
    pub mute: Flag,
}
/// Requires 100 columns and exactly three rows. Slots are presentation, never engine IDs.
pub fn draw(area: Rect, b: &mut Buffer, keys: &[Key; 25]) -> bool {
    if area.width < 100 || area.height != 3 {
        return false;
    }
    for (i, key) in keys.iter().enumerate() {
        let black = i < 24 && [1, 3, 6, 8, 10].contains(&(i % 12));
        let fg = if black { Color::White } else { Color::Black };
        let bg = if !key.available {
            Color::DarkGray
        } else if key.selected {
            Color::Green
        } else if black {
            Color::Black
        } else {
            Color::Rgb(255, 255, 210)
        };
        let x = area.x + i as u16 * 4;
        let label = if i == 24 {
            'M'
        } else {
            char::from_digit((key.number % 10) as u32, 10).unwrap()
        };
        let rows = [
            format!(
                "{}{} │",
                if key.selected { '>' } else { ' ' },
                if key.available { label } else { '-' }
            ),
            format!(
                "{}{}{}│",
                key.recording.symbol('R'),
                key.solo.symbol('S'),
                key.mute.symbol('M')
            ),
            "   │".into(),
        ];
        for (row, line) in rows.iter().enumerate() {
            let style = Style::default()
                .fg(fg)
                .bg(if row == 2 && black && !key.selected {
                    Color::Rgb(255, 255, 210)
                } else {
                    bg
                });
            b.set_stringn(x, area.y + row as u16, line, 4, style);
        }
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simultaneous_flags_and_master() {
        let k = Key {
            number: 1,
            available: true,
            selected: true,
            recording: Flag::On,
            solo: Flag::Unknown,
            mute: Flag::On,
        };
        let mut b = Buffer::empty(Rect::new(0, 0, 100, 3));
        assert!(draw(b.area, &mut b, &[k; 25]));
        assert_eq!(b[(0, 1)].symbol(), "R");
        assert_eq!(b[(1, 1)].symbol(), "?");
        assert_eq!(b[(2, 1)].symbol(), "M");
        assert_eq!(b[(97, 0)].symbol(), "M");
        assert_eq!(b[(0, 0)].symbol(), ">");
    }
}
