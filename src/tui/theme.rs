use ratatui::style::Color;

pub struct Theme {
    pub bg: Color,
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
            bg: Color::Rgb(16, 20, 28),
            user_border: Color::Rgb(92, 62, 148),
            assistant_border: Color::Rgb(100, 13, 95),
            kb_border: Color::DarkGray,
            system_border: Color::Yellow,
            input_active: Color::Rgb(242, 89, 18),
            input_disabled: Color::DarkGray,
            accent: Color::Rgb(242, 89, 18),
            status_busy: Color::Yellow,
            status_ok: Color::Green,
        }
    }
}
