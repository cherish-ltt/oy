use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    /// Background of chat/input surface areas
    pub surface_bg: Color,
    /// Default foreground on surface
    pub surface_fg: Color,
    /// User message foreground
    pub user_fg: Color,
    /// User message background (fills full line)
    pub user_bg: Color,
    /// Assistant message foreground
    pub assistant_fg: Color,
    /// Assistant message background (fills full line)
    pub assistant_bg: Color,
    /// Tool message foreground
    pub tool_fg: Color,
    /// Tool message background (fills full line)
    pub tool_bg: Color,
    /// Borders, frames
    pub border: Color,
    /// Input box border
    pub input_border: Color,
    /// Accent / highlight (cyan-like)
    pub accent: Color,
    /// Subtle secondary text (dark-gray-like)
    pub subtle: Color,
    /// Success / completion (green-like)
    pub success: Color,
    /// Error / removed text (red-like)
    pub error: Color,
    /// Warning / attention (yellow-like)
    pub warning: Color,
    /// Inline code / code block foreground
    pub code_fg: Color,
    /// Inline code / code block background
    pub code_bg: Color,
    /// Status bar foreground
    pub status_fg: Color,
    /// Status bar background
    pub status_bg: Color,
    /// Information / UI message color
    pub info_fg: Color,
    /// Link text color
    pub link_fg: Color,
}

pub static DARK_THEME: Theme = Theme {
    name: "dark",
    surface_bg: Color::Black,
    surface_fg: Color::White,
    user_fg: Color::LightBlue,
    user_bg: Color::Rgb(25, 30, 50),
    assistant_fg: Color::LightGreen,
    assistant_bg: Color::Rgb(20, 30, 20),
    tool_fg: Color::LightMagenta,
    tool_bg: Color::Rgb(40, 20, 40),
    border: Color::DarkGray,
    input_border: Color::Blue,
    accent: Color::Cyan,
    subtle: Color::DarkGray,
    success: Color::Green,
    error: Color::Red,
    warning: Color::Yellow,
    code_fg: Color::Cyan,
    code_bg: Color::Rgb(30, 30, 30),
    status_fg: Color::Gray,
    status_bg: Color::Black,
    info_fg: Color::LightBlue,
    link_fg: Color::LightBlue,
};

pub static LIGHT_THEME: Theme = Theme {
    name: "light",
    surface_bg: Color::White,
    surface_fg: Color::Black,
    user_fg: Color::Blue,
    user_bg: Color::Rgb(235, 240, 255),
    assistant_fg: Color::Green,
    assistant_bg: Color::Rgb(235, 255, 235),
    tool_fg: Color::Magenta,
    tool_bg: Color::Rgb(255, 235, 255),
    border: Color::DarkGray,
    input_border: Color::Blue,
    accent: Color::Cyan,
    subtle: Color::DarkGray,
    success: Color::Green,
    error: Color::Red,
    warning: Color::Yellow,
    code_fg: Color::Cyan,
    code_bg: Color::Rgb(240, 240, 240),
    status_fg: Color::DarkGray,
    status_bg: Color::White,
    info_fg: Color::Blue,
    link_fg: Color::Blue,
};
