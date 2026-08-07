use std::path::PathBuf;

pub struct Context {
    pub cwd: PathBuf,
    pub term_width: usize,
}

pub enum Dynamic<T> {
    Static(T),
    Dynamic(Box<dyn Fn(&Context) -> T>),
}

impl<T: Clone> Dynamic<T> {
    pub fn resolve(&self, ctx: &Context) -> T {
        match self {
            Dynamic::Static(v) => v.clone(),
            Dynamic::Dynamic(f) => f(ctx),
        }
    }
}

impl<T> From<T> for Dynamic<T> {
    fn from(v: T) -> Self {
        Dynamic::Static(v)
    }
}

impl<T> Dynamic<T> {
    pub fn dynamic(f: impl Fn(&Context) -> T + 'static) -> Self {
        Dynamic::Dynamic(Box::new(f))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Color {
    Rgb(u8, u8, u8),
    Ansi256(u8),
    Reset,
}

impl Color {
    pub fn from_hex(hex: &str) -> Self {
        let hex = hex.trim_start_matches('#');
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        Color::Rgb(r, g, b)
    }

    pub fn to_ansi(self) -> String {
        match self {
            Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
            Color::Ansi256(n) => format!("\x1b[38;5;{n}m"),
            Color::Reset => "\x1b[0m".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub text: String,
    pub color: Option<Color>,
}

impl Segment {
    pub fn new(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            color: Some(color),
        }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: None,
        }
    }
}

pub fn render_line(segments: &[Segment]) -> String {
    let mut out = String::new();
    for seg in segments {
        if let Some(color) = seg.color {
            out.push_str(&color.to_ansi());
        }
        out.push_str(&seg.text);
    }
    out.push_str(&Color::Reset.to_ansi());
    out
}

pub struct Component<D> {
    pub enabled: Dynamic<bool>,
    pub get_data: Box<dyn Fn(&Context) -> D>,
    pub render: Box<dyn Fn(&D) -> Vec<Segment>>,
    pub file_patterns: Vec<String>,
}

impl<D> Component<D> {
    pub fn render_default(&self, ctx: &Context) -> Vec<Segment> {
        let data = (self.get_data)(ctx);
        (self.render)(&data)
    }
}
