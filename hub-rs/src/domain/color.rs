#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color::new(0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// `brightness` is 0-100. Rounds up so that a dim color never turns black.
    pub fn with_brightness(self, brightness: f64) -> Color {
        let ch = |v: u8| (v as f64 * brightness / 100.0).ceil().clamp(0.0, 255.0) as u8;
        Color::new(ch(self.r), ch(self.g), ch(self.b))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSchemeId {
    Default,
    YellowPink,
}

impl ColorSchemeId {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorSchemeId::Default => "default",
            ColorSchemeId::YellowPink => "yellow-pink",
        }
    }

    pub fn parse(s: &str) -> Option<ColorSchemeId> {
        match s {
            "default" => Some(ColorSchemeId::Default),
            "yellow-pink" => Some(ColorSchemeId::YellowPink),
            _ => None,
        }
    }

    pub fn scheme(self) -> &'static ColorScheme {
        match self {
            ColorSchemeId::Default => &DEFAULT_SCHEME,
            ColorSchemeId::YellowPink => &YELLOW_PINK_SCHEME,
        }
    }
}

pub struct ColorScheme {
    pub program: Color,
    pub preview: Color,
    pub highlight: Color,
    pub unknown: Color,
    pub idle: Color,
}

const DEFAULT_SCHEME: ColorScheme = ColorScheme {
    program: Color::new(255, 0, 0),
    preview: Color::new(0, 255, 0),
    highlight: Color::new(255, 255, 255),
    unknown: Color::new(0, 0, 255),
    idle: Color::new(0, 1, 0),
};

/// Better contrast for the red-green color blind.
const YELLOW_PINK_SCHEME: ColorScheme = ColorScheme {
    program: Color::new(255, 255, 0),
    preview: Color::new(255, 0, 255),
    highlight: Color::new(255, 255, 255),
    unknown: Color::new(0, 0, 255),
    idle: Color::new(0, 1, 0),
};
