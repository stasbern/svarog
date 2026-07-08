use ratatui::style::Color;

pub struct Theme {
    pub user_border: Color,
    pub assistant_border: Color,
    pub kb_border: Color,
    pub system_border: Color,
    pub input_active: Color,
    pub input_disabled: Color,
    pub accent: Color,
    pub status_busy: Color,
    pub status_ok: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            user_border:    Color::Rgb(145, 10, 103),
            assistant_border: Color::Rgb(60, 7, 83),
            kb_border:      Color::DarkGray,
            system_border:  Color::Yellow,
            input_active:   Color::Yellow,
            input_disabled: Color::DarkGray,
            accent:         Color::Rgb(60, 7, 83),
            status_busy:    Color::Yellow,
            status_ok:      Color::Green,
        }
    }
}