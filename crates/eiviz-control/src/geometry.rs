use crate::session::MultiviewTemplate;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MultiviewPane {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl MultiviewTemplate {
    pub fn panes(self) -> Vec<MultiviewPane> {
        match self {
            Self::PreviewProgram2 => {
                let mut panes = vec![pane(0.0, 0.0, 0.5, 0.5), pane(0.5, 0.0, 0.5, 0.5)];
                add_grid(&mut panes, 2, 1, 0.0, 0.5, 1.0, 0.5);
                panes
            }
            Self::PreviewProgram8 => {
                let mut panes = vec![pane(0.0, 0.0, 0.5, 0.5), pane(0.5, 0.0, 0.5, 0.5)];
                add_grid(&mut panes, 4, 2, 0.0, 0.5, 1.0, 0.5);
                panes
            }
            Self::PreviewProgram8Bottom => {
                let mut panes = Vec::new();
                add_grid(&mut panes, 4, 2, 0.0, 0.0, 1.0, 0.5);
                panes.push(pane(0.0, 0.5, 0.5, 0.5));
                panes.push(pane(0.5, 0.5, 0.5, 0.5));
                panes
            }
            Self::PreviewProgram8Left => {
                let mut panes = vec![pane(0.0, 0.5, 0.5, 0.5), pane(0.0, 0.0, 0.5, 0.5)];
                add_grid(&mut panes, 2, 4, 0.5, 0.0, 0.5, 1.0);
                panes
            }
            Self::PreviewProgram8Right => {
                let mut panes = Vec::new();
                add_grid(&mut panes, 2, 4, 0.0, 0.0, 0.5, 1.0);
                panes.push(pane(0.5, 0.5, 0.5, 0.5));
                panes.push(pane(0.5, 0.0, 0.5, 0.5));
                panes
            }
            Self::Quad4TopLeft => quad4(0),
            Self::Quad4TopRight => quad4(1),
            Self::Quad4BottomLeft => quad4(2),
            Self::Quad4BottomRight => quad4(3),
            Self::Large5TopLeft => large5(0, 0),
            Self::Large5TopRight => large5(1, 0),
            Self::Large5BottomLeft => large5(0, 1),
            Self::Large5BottomRight => large5(1, 1),
            Self::Grid3x3 => {
                let mut panes = Vec::new();
                add_grid(&mut panes, 3, 3, 0.0, 0.0, 1.0, 1.0);
                panes
            }
            Self::Grid4x4 => {
                let mut panes = Vec::new();
                add_grid(&mut panes, 4, 4, 0.0, 0.0, 1.0, 1.0);
                panes
            }
            Self::Grid2x2 => {
                let mut panes = Vec::new();
                add_grid(&mut panes, 2, 2, 0.0, 0.0, 1.0, 1.0);
                panes
            }
        }
    }
}

fn pane(x: f32, y: f32, width: f32, height: f32) -> MultiviewPane {
    MultiviewPane {
        x,
        y,
        width,
        height,
    }
}

fn add_grid(panes: &mut Vec<MultiviewPane>, cols: i32, rows: i32, x: f32, y: f32, w: f32, h: f32) {
    for i in 0..cols * rows {
        let col = i % cols;
        let row = i / cols;
        let x0 = x + w * col as f32 / cols as f32;
        let y0 = y + h * row as f32 / rows as f32;
        let x1 = x + w * (col + 1) as f32 / cols as f32;
        let y1 = y + h * (row + 1) as f32 / rows as f32;
        panes.push(pane(x0, y0, x1 - x0, y1 - y0));
    }
}

fn quad4(small_quad: i32) -> Vec<MultiviewPane> {
    let mut panes = Vec::new();
    for quad in 0..4 {
        let x = (quad % 2) as f32 * 0.5;
        let y = (quad / 2) as f32 * 0.5;
        if quad == small_quad {
            add_grid(&mut panes, 2, 2, x, y, 0.5, 0.5);
        } else {
            panes.push(pane(x, y, 0.5, 0.5));
        }
    }
    panes
}

fn large5(large_col: i32, large_row: i32) -> Vec<MultiviewPane> {
    let x0 = large_col as f32 / 3.0;
    let y0 = large_row as f32 / 3.0;
    let x1 = (large_col + 2) as f32 / 3.0;
    let y1 = (large_row + 2) as f32 / 3.0;
    let mut panes = vec![pane(x0, y0, x1 - x0, y1 - y0)];
    for row in 0..3 {
        for col in 0..3 {
            if col >= large_col && col < large_col + 2 && row >= large_row && row < large_row + 2 {
                continue;
            }
            let sx0 = col as f32 / 3.0;
            let sy0 = row as f32 / 3.0;
            let sx1 = (col + 1) as f32 / 3.0;
            let sy1 = (row + 1) as f32 / 3.0;
            panes.push(pane(sx0, sy0, sx1 - sx0, sy1 - sy0));
        }
    }
    panes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_counts_match_windows_host() {
        assert_eq!(MultiviewTemplate::PreviewProgram8.panes().len(), 10);
        assert_eq!(MultiviewTemplate::PreviewProgram2.panes().len(), 4);
        assert_eq!(MultiviewTemplate::Quad4TopLeft.panes().len(), 7);
        assert_eq!(MultiviewTemplate::Large5TopLeft.panes().len(), 6);
        assert_eq!(MultiviewTemplate::Grid2x2.panes().len(), 4);
        assert_eq!(MultiviewTemplate::Grid3x3.panes().len(), 9);
        assert_eq!(MultiviewTemplate::Grid4x4.panes().len(), 16);
        assert_eq!(
            MultiviewTemplate::PreviewProgram8.panes().len(),
            MultiviewTemplate::PreviewProgram8.tile_count()
        );
    }
}
