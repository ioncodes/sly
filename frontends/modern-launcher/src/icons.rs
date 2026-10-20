use resvg::{tiny_skia, usvg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusIcon {
    Warning,
    Error,
}

impl StatusIcon {
    const BASE_ID: u16 = 0x8000;

    pub fn id(self) -> u16 {
        Self::BASE_ID + self as u16
    }

    pub fn from_id(id: u16) -> Option<Self> {
        match id.checked_sub(Self::BASE_ID)? {
            0 => Some(Self::Warning),
            1 => Some(Self::Error),
            _ => None,
        }
    }

    fn svg(self) -> &'static str {
        match self {
            Self::Warning => include_str!("../../../vendor/icons/lucide/triangle-alert.svg"),
            Self::Error => include_str!("../../../vendor/icons/lucide/circle-alert.svg"),
        }
    }

    pub fn rasterize(self, width: u16, height: u16) -> Option<Vec<u8>> {
        let tree = usvg::Tree::from_str(self.svg(), &usvg::Options::default()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new(width.into(), height.into())?;
        let size = tree.size();

        let transform = tiny_skia::Transform::from_scale(
            f32::from(width) / size.width(),
            f32::from(height) / size.height(),
        );

        resvg::render(&tree, transform, &mut pixmap.as_mut());
        Some(pixmap.pixels().iter().map(|pixel| pixel.alpha()).collect())
    }
}
