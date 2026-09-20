use std::{
    collections::VecDeque,
    io::{self, Stdout},
    num::NonZero,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use eternal_core::{Analysis, Audio, BranchGraph, PlaybackPlanner, Step, TransitionProbability};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Row, Table},
};
use rodio::{DeviceSinkBuilder, Player};

use crate::audio_source::BeatSource;

const QUEUED_BEATS: usize = 8;
const HISTORY_LENGTH: usize = 256;
const VOLUME_STEP: f32 = 0.05;
const MAX_VOLUME: f32 = 2.0;
const SEEK_SECONDS: f64 = 10.0;

const INK: Color = Color::Rgb(205, 214, 244);
const MUTED: Color = Color::Rgb(108, 112, 134);
const SURFACE: Color = Color::Rgb(49, 50, 68);
const VOLUME_TRACK: Color = Color::Rgb(69, 71, 90);
pub(crate) const CYAN: Color = Color::Rgb(137, 220, 235);
const GREEN: Color = Color::Rgb(166, 227, 161);
pub(crate) const MAGENTA: Color = Color::Rgb(203, 166, 247);
pub(crate) const YELLOW: Color = Color::Rgb(249, 226, 175);

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode().context("could not enable terminal raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("could not enter alternate screen")?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[derive(Clone)]
struct QueuedBeat {
    step: Step,
    probability: f32,
}

struct SessionClock {
    started: Instant,
    paused_at: Option<Instant>,
    paused_for: Duration,
}

impl SessionClock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            paused_at: None,
            paused_for: Duration::ZERO,
        }
    }

    fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    fn resume(&mut self) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_for += paused_at.elapsed();
        }
    }

    fn elapsed(&self) -> Duration {
        self.elapsed_at(Instant::now())
    }

    fn elapsed_at(&self, now: Instant) -> Duration {
        let current_pause = self
            .paused_at
            .map_or(Duration::ZERO, |paused_at| now.duration_since(paused_at));
        now.duration_since(self.started)
            .saturating_sub(self.paused_for + current_pause)
    }
}

struct Dashboard<'a> {
    analysis: &'a Analysis,
    graph: &'a BranchGraph,
    title: String,
    session_clock: SessionClock,
    live: Option<QueuedBeat>,
    queue: VecDeque<QueuedBeat>,
    history: VecDeque<QueuedBeat>,
    choices: Vec<TransitionProbability>,
    coverage: Vec<u32>,
    jump_count: u64,
    paused: bool,
    volume: f32,
}

impl<'a> Dashboard<'a> {
    fn new(analysis: &'a Analysis, graph: &'a BranchGraph, input: &Path, volume: f32) -> Self {
        Self {
            analysis,
            graph,
            title: input
                .file_name()
                .unwrap_or(input.as_os_str())
                .to_string_lossy()
                .into_owned(),
            session_clock: SessionClock::new(),
            live: None,
            queue: VecDeque::new(),
            history: VecDeque::new(),
            choices: Vec::new(),
            coverage: vec![0; analysis.beats.len()],
            jump_count: 0,
            paused: false,
            volume,
        }
    }

    fn record_finished(&mut self, finished: QueuedBeat) {
        if let Some(visits) = self.coverage.get_mut(finished.step.beat) {
            *visits = visits.saturating_add(1);
        }
        if finished.step.jumped_from.is_some() {
            self.jump_count += 1;
        }
        self.history.push_front(finished.clone());
        self.history.truncate(HISTORY_LENGTH);
        self.live = self.queue.front().cloned().or(Some(finished));
    }
}

pub fn play(
    audio: &Audio,
    analysis: &Analysis,
    graph: &BranchGraph,
    seed: Option<u64>,
    input: &Path,
) -> Result<()> {
    let mut output = DeviceSinkBuilder::open_default_sink()
        .context("could not open the default audio output device")?;
    output.log_on_drop(false);
    let player = Player::connect_new(output.mixer());
    NonZero::new(audio.channels).context("audio has no channels")?;
    NonZero::new(audio.sample_rate).context("audio has no sample rate")?;
    let mut planner = seed.map_or_else(
        || PlaybackPlanner::new(graph.clone()),
        |seed| PlaybackPlanner::with_seed(graph.clone(), seed),
    );
    let mut pending = plan_next(&mut planner)?;
    let mut dashboard = Dashboard::new(analysis, graph, input, player.volume());
    let mut terminal = TerminalGuard::new()?;

    loop {
        while player.len() < QUEUED_BEATS {
            let next = plan_next(&mut planner)?;
            let beat = &analysis.beats[pending.step.beat];
            let source = BeatSource::new(
                audio,
                beat.start,
                beat.duration,
                pending.step.jumped_from.is_some(),
                next.step.jumped_from.is_some(),
            );
            player.append(source);
            dashboard.queue.push_back(pending);
            pending = next;
        }

        while dashboard.queue.len() > player.len().max(1) {
            if let Some(finished) = dashboard.queue.pop_front() {
                dashboard.record_finished(finished);
            }
        }
        if dashboard.live.is_none() {
            dashboard.live = dashboard.queue.front().cloned();
        }
        dashboard.choices = planner.next_probabilities();
        terminal.terminal.draw(|frame| draw(frame, &dashboard))?;

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                return Ok(());
            }
            if key.code == KeyCode::Char('p') {
                dashboard.paused = !dashboard.paused;
                if dashboard.paused {
                    dashboard.session_clock.pause();
                    player.pause();
                } else {
                    dashboard.session_clock.resume();
                    player.play();
                }
            }
            if matches!(key.code, KeyCode::Char(',' | '.')) {
                let direction = if key.code == KeyCode::Char('.') {
                    1.0
                } else {
                    -1.0
                };
                dashboard.volume =
                    (dashboard.volume + direction * VOLUME_STEP).clamp(0.0, MAX_VOLUME);
                player.set_volume(dashboard.volume);
            }
            if matches!(key.code, KeyCode::Char('f' | 'b')) {
                let current = dashboard.live.as_ref().map_or(0, |item| item.step.beat);
                let target = seek_beat(
                    analysis,
                    current,
                    if key.code == KeyCode::Char('f') {
                        SEEK_SECONDS
                    } else {
                        -SEEK_SECONDS
                    },
                );
                let _ = planner.continue_from(target);
                pending = QueuedBeat {
                    step: Step {
                        beat: target,
                        jumped_from: None,
                    },
                    probability: 1.0,
                };
                player.clear();
                if !dashboard.paused {
                    player.play();
                }
                dashboard.queue.clear();
                dashboard.live = Some(pending.clone());
                dashboard.choices = planner.next_probabilities();
            }
        }
    }
}

fn plan_next(planner: &mut PlaybackPlanner) -> Result<QueuedBeat> {
    let choices = planner.next_probabilities();
    let step = planner
        .next_step()
        .context("the playback graph contains no beats")?;
    Ok(QueuedBeat {
        probability: selected_probability(&choices, &step),
        step,
    })
}

fn seek_beat(analysis: &Analysis, current: usize, offset_seconds: f64) -> usize {
    let current_time = analysis.beats.get(current).map_or(0.0, |beat| beat.start);
    let target_time = (current_time + offset_seconds).max(0.0);
    analysis
        .beats
        .partition_point(|beat| beat.start <= target_time)
        .saturating_sub(1)
        .min(analysis.beats.len().saturating_sub(1))
}

fn selected_probability(choices: &[TransitionProbability], step: &Step) -> f32 {
    choices
        .iter()
        .find(|choice| {
            choice.destination == step.beat && choice.is_sequential == step.jumped_from.is_none()
        })
        .map_or(1.0, |choice| choice.probability)
}

fn draw(frame: &mut Frame, dashboard: &Dashboard<'_>) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length((frame.area().height / 3).clamp(5, 15)),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());
    draw_header(frame, outer[0], dashboard);
    draw_position(frame, outer[1], dashboard);
    let middle = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(outer[2]);
    draw_stitch(frame, middle[0], dashboard);
    let details = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(middle[1]);
    draw_history(frame, details[0], dashboard);
    draw_choices(frame, details[1], dashboard);
    draw_footer(frame, outer[3], dashboard);
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE))
}

fn draw_header(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let live = dashboard.live.as_ref().map_or(0, |beat| beat.step.beat);
    let title = Line::from(vec![
        Span::styled(
            " ♫  ETERNAL JUKEBOX ",
            Style::default()
                .fg(Color::Black)
                .bg(CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            &dashboard.title,
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ),
    ]);
    let (state, state_color) = if dashboard.paused {
        ("  PAUSED  ", YELLOW)
    } else {
        ("  PLAYING  ", GREEN)
    };
    let status = Line::from(vec![
        Span::styled(
            state,
            Style::default()
                .fg(Color::Black)
                .bg(state_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("BEAT {live:03}"),
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  infinite mix", Style::default().fg(MUTED)),
    ]);
    frame.render_widget(
        Paragraph::new(vec![title, status]).block(panel("NOW PLAYING")),
        area,
    );
    let volume_width = 28.min(area.width.saturating_sub(2));
    if volume_width > 0 && area.height > 2 {
        let bar_width = 14;
        let filled = ((dashboard.volume / MAX_VOLUME) * bar_width as f32).round() as usize;
        let volume = Line::from(vec![
            Span::styled("VOL ", Style::default().fg(YELLOW)),
            Span::styled("█".repeat(filled), Style::default().fg(YELLOW)),
            Span::styled(
                "░".repeat(bar_width - filled),
                Style::default().fg(VOLUME_TRACK),
            ),
            Span::styled(
                format!(" {:>3}%", (dashboard.volume * 100.0).round() as u16),
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(volume).alignment(Alignment::Right),
            Rect::new(
                area.right().saturating_sub(volume_width + 1),
                area.y + 1,
                volume_width,
                1,
            ),
        );
    }
}

fn draw_position(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let beat = dashboard.live.as_ref().map_or(0, |item| item.step.beat);
    let count = dashboard.analysis.beats.len().max(1);
    let ratio = beat as f64 / count.saturating_sub(1).max(1) as f64;
    let time = dashboard
        .analysis
        .beats
        .get(beat)
        .map_or(0.0, |item| item.start);
    let label = format!(
        " {:02}:{:02}  ·  beat {}/{}  ·  {:.1} BPM ",
        time as u64 / 60,
        time as u64 % 60,
        beat,
        count - 1,
        dashboard.analysis.tempo
    );
    let block = panel("BEAT CONNECTIONS · TRACK POSITION");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let map_height = inner.height.saturating_sub(2);
    let next_jump = dashboard
        .queue
        .iter()
        .skip(1)
        .find_map(|item| item.step.jumped_from.map(|source| (source, item.step.beat)));
    crate::connections::draw(
        frame,
        Rect::new(inner.x, inner.y, inner.width, map_height),
        dashboard.graph,
        count,
        beat,
        next_jump,
    );
    let gauge_y = inner.y + map_height;
    frame.render_widget(
        Gauge::default()
            .gauge_style(
                Style::default()
                    .fg(CYAN)
                    .bg(SURFACE)
                    .add_modifier(Modifier::BOLD),
            )
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label),
        Rect::new(inner.x, gauge_y, inner.width, 1),
    );
    if inner.height >= 2 {
        draw_coverage(
            frame,
            Rect::new(inner.x, gauge_y + 1, inner.width, 1),
            &dashboard.coverage,
        );
    }
}

fn draw_coverage(frame: &mut Frame, area: Rect, coverage: &[u32]) {
    if area.width == 0 || coverage.is_empty() {
        return;
    }
    let width = usize::from(area.width);
    let bins: Vec<u32> = (0..width)
        .map(|column| {
            let start = column * coverage.len() / width;
            let end = ((column + 1) * coverage.len() / width).max(start + 1);
            coverage[start.min(coverage.len() - 1)..end.min(coverage.len())]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
        })
        .collect();
    let maximum = bins.iter().copied().max().unwrap_or(0).max(1) as f32;
    let spans = bins.into_iter().map(|visits| {
        let intensity = (visits as f32 / maximum).sqrt();
        let color = if visits == 0 {
            VOLUME_TRACK
        } else {
            Color::Rgb(
                (70.0 + 185.0 * intensity) as u8,
                (90.0 + 90.0 * (1.0 - intensity)) as u8,
                (210.0 - 120.0 * intensity) as u8,
            )
        };
        Span::styled("█", Style::default().fg(color))
    });
    frame.render_widget(Paragraph::new(Line::from(spans.collect::<Vec<_>>())), area);
}

fn draw_stitch(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let block = panel("LIVE STITCH");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let number_width = dashboard
        .analysis
        .beats
        .len()
        .saturating_sub(1)
        .to_string()
        .len();
    let marker_width = u16::try_from(number_width + 2)
        .unwrap_or(u16::MAX)
        .min(inner.width);
    let mut stitch_width = inner.width.saturating_mul(4) / 5;
    stitch_width = stitch_width.max(marker_width);
    if stitch_width.saturating_sub(marker_width) % 2 != 0 {
        stitch_width = stitch_width.saturating_sub(1).max(marker_width);
    }
    let stitch_x = inner.x + inner.width.saturating_sub(stitch_width) / 2;
    let side_width = stitch_width.saturating_sub(marker_width) / 2;
    let history_area = Rect::new(stitch_x, inner.y, side_width, 1);
    let live_area = Rect::new(stitch_x + side_width, inner.y, marker_width, 1);
    let queue_area = Rect::new(live_area.x + live_area.width, inner.y, side_width, 1);

    let mut queue_spans = Vec::new();
    for item in dashboard.queue.iter().skip(1) {
        let arrow = if item.step.jumped_from.is_some() {
            " ⇢ "
        } else {
            " ─ "
        };
        let mut next = queue_spans.clone();
        next.push(Span::styled(
            arrow,
            Style::default().fg(if item.step.jumped_from.is_some() {
                MAGENTA
            } else {
                SURFACE
            }),
        ));
        next.push(Span::styled(
            format!("{:0number_width$}", item.step.beat),
            Style::default().fg(MUTED),
        ));
        if Line::from(next.clone()).width() > usize::from(queue_area.width) {
            break;
        }
        queue_spans = next;
    }

    let mut history_spans = Vec::new();
    for item in &dashboard.history {
        let mut next = vec![
            Span::styled(
                format!("{:0number_width$}", item.step.beat),
                Style::default().fg(if item.step.jumped_from.is_some() {
                    MAGENTA
                } else {
                    MUTED
                }),
            ),
            Span::styled(
                if history_spans.is_empty() {
                    " ━▶ "
                } else {
                    " ─ "
                },
                Style::default().fg(if history_spans.is_empty() {
                    CYAN
                } else {
                    SURFACE
                }),
            ),
        ];
        next.extend(history_spans.iter().cloned());
        if Line::from(next.clone()).width() > usize::from(history_area.width) {
            break;
        }
        history_spans = next;
    }

    frame.render_widget(
        Paragraph::new(Line::from(history_spans)).alignment(Alignment::Right),
        history_area,
    );
    if let Some(live) = &dashboard.live {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("[{:0number_width$}]", live.step.beat),
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
            live_area,
        );
    }
    frame.render_widget(Paragraph::new(Line::from(queue_spans)), queue_area);
}

fn draw_history(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let rows = dashboard.history.iter().filter_map(|item| {
        item.step.jumped_from.map(|source| {
            Row::new(vec![
                format!("{source} → {}", item.step.beat),
                format!("{:.1}%", item.probability * 100.0),
                jump_direction(source, item.step.beat).to_owned(),
            ])
            .style(Style::default().fg(MAGENTA))
        })
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(46),
            Constraint::Percentage(27),
            Constraint::Percentage(27),
        ],
    )
    .header(
        Row::new(["transition", "chosen odds", "direction"])
            .style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
    )
    .block(panel("RECENT JUMPS"));
    frame.render_widget(table, area);
}

fn draw_choices(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let mut choices = dashboard.choices.clone();
    choices.sort_by(|left, right| right.probability.total_cmp(&left.probability));
    let width = usize::from(area.width.saturating_sub(21)).min(24);
    let items = choices
        .into_iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|choice| {
            let filled = (choice.probability * width as f32).round() as usize;
            let bar = format!(
                "{}{}",
                "█".repeat(filled),
                "░".repeat(width.saturating_sub(filled))
            );
            let label = if choice.is_sequential {
                format!("continue → {}", choice.destination)
            } else {
                format!("{} → {}", choice.source, choice.destination)
            };
            let distance = choice
                .distance
                .map_or_else(String::new, |value| format!("  d={value:.2}"));
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{label:<15}"),
                    Style::default().fg(if choice.is_sequential { MUTED } else { MAGENTA }),
                ),
                Span::styled(
                    bar,
                    Style::default().fg(if choice.is_sequential { CYAN } else { MAGENTA }),
                ),
                Span::styled(
                    format!(" {:>5.1}%{distance}", choice.probability * 100.0),
                    Style::default().fg(INK),
                ),
            ]))
        });
    frame.render_widget(
        List::new(items).block(panel("PLANNER • NEXT DECISION ODDS")),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let elapsed = dashboard.session_clock.elapsed().as_secs();
    let text = Line::from(vec![
        Span::styled(
            format!(
                "  ◉  {:02}:{:02}:{:02}",
                elapsed / 3600,
                elapsed / 60 % 60,
                elapsed % 60
            ),
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("    {} jumps", dashboard.jump_count),
            Style::default().fg(MAGENTA),
        ),
        Span::styled(
            format!("    {} transitions", dashboard.graph.branch_count()),
            Style::default().fg(CYAN),
        ),
        Span::styled(
            "    ◀ b  seek  f ▶     − ,  volume  . +     p  pause     q  quit  ",
            Style::default().fg(YELLOW),
        ),
    ]);
    frame.render_widget(Paragraph::new(text).block(panel("SESSION")), area);
}

fn jump_direction(source: usize, destination: usize) -> &'static str {
    if destination < source {
        "backward"
    } else {
        "forward"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_clock_does_not_advance_while_paused() {
        let started = Instant::now();
        let clock = SessionClock {
            started,
            paused_at: Some(started + Duration::from_secs(5)),
            paused_for: Duration::ZERO,
        };

        assert_eq!(
            clock.elapsed_at(started + Duration::from_secs(20)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn session_clock_excludes_completed_pauses() {
        let started = Instant::now();
        let clock = SessionClock {
            started,
            paused_at: None,
            paused_for: Duration::from_secs(7),
        };

        assert_eq!(
            clock.elapsed_at(started + Duration::from_secs(20)),
            Duration::from_secs(13)
        );
    }
}
