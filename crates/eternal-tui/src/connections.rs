use eternal_core::BranchGraph;
use ratatui::{
    Frame,
    layout::Rect,
    style::Color,
    symbols::Marker,
    widgets::canvas::{Canvas, Context, Line},
};

use crate::tui::{CYAN, MAGENTA, YELLOW};

/// Shared beat axis: the first and last beats sit at the ends of the gauge.
fn position(beat: usize, count: usize) -> f64 {
    beat.min(count.saturating_sub(1)) as f64 / count.saturating_sub(1).max(1) as f64
}

fn connection(context: &mut Context<'_>, source: f64, destination: f64, color: Color) {
    // The automatic end-to-start wrap is playback behavior, not a visible path.
    if source >= 1.0 && destination <= 0.0 {
        return;
    }
    let height = (destination - source).abs().sqrt() * 1.18;
    let mut previous = (source, 0.0);
    for segment in 1..=48 {
        let t = f64::from(segment) / 48.0;
        let point = (
            source + (destination - source) * t * t * (3.0 - 2.0 * t),
            3.0 * (1.0 - t) * t * height,
        );
        context.draw(&Line {
            x1: previous.0,
            y1: previous.1,
            x2: point.0,
            y2: point.1,
            color,
        });
        previous = point;
    }
}

pub(crate) fn draw(
    frame: &mut Frame,
    area: Rect,
    graph: &BranchGraph,
    count: usize,
    live: usize,
    next_jump: Option<(usize, usize)>,
) {
    if area.is_empty() || count == 0 {
        return;
    }
    // Fit the visible cubic peaks, excluding the hidden wrap, with 4% headroom.
    let max_span = graph
        .branches
        .iter()
        .enumerate()
        .flat_map(|(source, branches)| branches.iter().map(move |b| (source, b.destination)))
        .chain(next_jump)
        .filter(|&(source, destination)| !(source == count - 1 && destination == 0))
        .map(|(source, destination)| (position(source, count) - position(destination, count)).abs())
        .fold(0.0_f64, f64::max);
    let ceiling = (max_span.sqrt() * 1.18 * 0.75 / 0.96).max(0.01);
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, 1.0])
        .y_bounds([0.0, ceiling])
        .paint(|context| {
            // Deduplicate connections at terminal resolution so dense tracks stay cheap.
            let columns = usize::from(area.width) * 2;
            let mut seen = std::collections::HashSet::new();
            for (source, branches) in graph.branches.iter().enumerate() {
                for branch in branches {
                    let a = position(source, count);
                    let b = position(branch.destination, count);
                    let endpoints = (
                        (a.min(b) * columns as f64) as usize,
                        (a.max(b) * columns as f64) as usize,
                    );
                    if seen.insert(endpoints) {
                        connection(context, a, b, Color::Rgb(48, 53, 73));
                    }
                }
            }
            context.layer();
            let cursor = position(live, count);
            context.draw(&Line {
                x1: cursor,
                y1: 0.0,
                x2: cursor,
                y2: ceiling * 0.96,
                color: CYAN,
            });
            context.layer();
            if let Some(branches) = graph.branches.get(live) {
                for branch in branches {
                    connection(
                        context,
                        cursor,
                        position(branch.destination, count),
                        MAGENTA,
                    );
                }
            }
            context.layer();
            if let Some((source, destination)) = next_jump {
                connection(
                    context,
                    position(source, count),
                    position(destination, count),
                    YELLOW,
                );
            }
        });
    frame.render_widget(canvas, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eternal_core::Branch;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    #[allow(clippy::float_cmp)] // These axis endpoints must be exact.
    fn beat_axis_includes_both_track_endpoints() {
        assert_eq!(position(0, 100), 0.0);
        assert_eq!(position(99, 100), 1.0);
        assert_eq!(position(50, 101), 0.5);
        assert_eq!(position(0, 0), 0.0);
        assert_eq!(position(0, 1), 0.0);
    }

    #[test]
    fn connections_render_at_small_and_normal_sizes() {
        let graph = BranchGraph {
            branches: vec![
                vec![Branch {
                    destination: 3,
                    distance: 0.1,
                }],
                vec![],
                vec![],
                vec![],
            ],
            threshold: 0.2,
            last_branch_point: 0,
        };
        for (width, height) in [(1, 1), (2, 2), (80, 10)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| draw(frame, frame.area(), &graph, 4, 0, Some((0, 3))))
                .unwrap();
            if width == 80 {
                assert!(
                    terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .any(|cell| cell.fg == YELLOW)
                );
            }
        }
    }
}
