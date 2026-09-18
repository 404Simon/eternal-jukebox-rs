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
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Row, Table},
};
use rodio::{DeviceSinkBuilder, Player, buffer::SamplesBuffer};

use crate::playback::beat_samples;

const QUEUED_BEATS: usize = 8;
const HISTORY_LENGTH: usize = 18;

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

struct Dashboard<'a> {
    analysis: &'a Analysis,
    graph: &'a BranchGraph,
    title: String,
    started: Instant,
    live: Option<QueuedBeat>,
    queue: VecDeque<QueuedBeat>,
    history: VecDeque<QueuedBeat>,
    choices: Vec<TransitionProbability>,
    jump_count: u64,
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
    let channels = NonZero::new(audio.channels).context("audio has no channels")?;
    let sample_rate = NonZero::new(audio.sample_rate).context("audio has no sample rate")?;
    let mut planner = seed.map_or_else(
        || PlaybackPlanner::new(graph.clone()),
        |seed| PlaybackPlanner::with_seed(graph.clone(), seed),
    );
    let first_choices = planner.next_probabilities();
    let first = planner
        .next_step()
        .context("the playback graph contains no beats")?;
    let first_probability = selected_probability(&first_choices, &first);
    let mut pending = QueuedBeat {
        step: first,
        probability: first_probability,
    };
    let mut dashboard = Dashboard {
        analysis,
        graph,
        title: input
            .file_name()
            .unwrap_or(input.as_os_str())
            .to_string_lossy()
            .into_owned(),
        started: Instant::now(),
        live: None,
        queue: VecDeque::new(),
        history: VecDeque::new(),
        choices: Vec::new(),
        jump_count: 0,
    };
    let mut terminal = TerminalGuard::new()?;

    loop {
        while player.len() < QUEUED_BEATS {
            let choices = planner.next_probabilities();
            let next = planner
                .next_step()
                .context("the playback graph contains no beats")?;
            let beat = &analysis.beats[pending.step.beat];
            let samples = beat_samples(
                audio,
                beat.start,
                beat.duration,
                pending.step.jumped_from.is_some(),
                next.jumped_from.is_some(),
            );
            player.append(SamplesBuffer::new(channels, sample_rate, samples));
            dashboard.queue.push_back(pending);
            let probability = selected_probability(&choices, &next);
            pending = QueuedBeat {
                step: next,
                probability,
            };
        }

        while dashboard.queue.len() > player.len().max(1) {
            if let Some(finished) = dashboard.queue.pop_front() {
                if finished.step.jumped_from.is_some() {
                    dashboard.jump_count += 1;
                }
                dashboard.history.push_front(finished.clone());
                dashboard.history.truncate(HISTORY_LENGTH);
                dashboard.live = dashboard.queue.front().cloned().or(Some(finished));
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
            && (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            return Ok(());
        }
    }
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
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());
    draw_header(frame, outer[0], dashboard);
    draw_position(frame, outer[1], dashboard);
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(outer[2]);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(middle[0]);
    draw_stitch(frame, left[0], dashboard);
    draw_history(frame, left[1], dashboard);
    draw_choices(frame, middle[1], dashboard);
    draw_footer(frame, outer[3], dashboard);
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn draw_header(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let live = dashboard.live.as_ref().map_or(0, |beat| beat.step.beat);
    let text = Line::from(vec![
        Span::styled(
            " ◉ ETERNAL JUKEBOX ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            &dashboard.title,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  •  "),
        Span::styled(
            format!("LIVE BEAT {live}"),
            Style::default().fg(Color::LightGreen),
        ),
    ]);
    frame.render_widget(Paragraph::new(text).block(panel("NOW PLAYING")), area);
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
        "{:02}:{:02}  beat {}/{}  •  {:.1} BPM",
        time as u64 / 60,
        time as u64 % 60,
        beat,
        count - 1,
        dashboard.analysis.tempo
    );
    frame.render_widget(
        Gauge::default()
            .block(panel("TRACK POSITION"))
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label),
        area,
    );
}

fn draw_stitch(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let mut spans = Vec::new();
    for (index, item) in dashboard.history.iter().rev().take(7).enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ─ ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            item.step.beat.to_string(),
            Style::default().fg(if item.step.jumped_from.is_some() {
                Color::Magenta
            } else {
                Color::Gray
            }),
        ));
    }
    if !spans.is_empty() {
        spans.push(Span::styled(" ═▶ ", Style::default().fg(Color::Cyan)));
    }
    if let Some(live) = &dashboard.live {
        spans.push(Span::styled(
            format!("[{}]", live.step.beat),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    }
    for item in dashboard.queue.iter().skip(1).take(5) {
        let arrow = if item.step.jumped_from.is_some() {
            " ⇢ "
        } else {
            " ─ "
        };
        spans.push(Span::styled(
            arrow,
            Style::default().fg(if item.step.jumped_from.is_some() {
                Color::Magenta
            } else {
                Color::DarkGray
            }),
        ));
        spans.push(Span::styled(
            item.step.beat.to_string(),
            Style::default().fg(Color::Gray),
        ));
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(spans),
            Line::raw(""),
            Line::styled(
                "magenta = jump   green = audible   gray = queued",
                Style::default().fg(Color::DarkGray),
            ),
        ])
        .alignment(Alignment::Center)
        .block(panel("LIVE STITCH")),
        area,
    );
}

fn draw_history(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let rows = dashboard.history.iter().filter_map(|item| {
        item.step.jumped_from.map(|source| {
            Row::new(vec![
                format!("{source} → {}", item.step.beat),
                format!("{:.1}%", item.probability * 100.0),
                jump_direction(source, item.step.beat).to_owned(),
            ])
            .style(Style::default().fg(Color::Magenta))
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
        Row::new(["transition", "chosen odds", "direction"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
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
                    Style::default().fg(if choice.is_sequential {
                        Color::Gray
                    } else {
                        Color::LightMagenta
                    }),
                ),
                Span::styled(
                    bar,
                    Style::default().fg(if choice.is_sequential {
                        Color::Cyan
                    } else {
                        Color::Magenta
                    }),
                ),
                Span::styled(
                    format!(" {:>5.1}%{distance}", choice.probability * 100.0),
                    Style::default().fg(Color::White),
                ),
            ]))
        });
    frame.render_widget(
        List::new(items).block(panel("PLANNER • NEXT DECISION ODDS")),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, dashboard: &Dashboard<'_>) {
    let elapsed = dashboard.started.elapsed().as_secs();
    let text = format!(
        "  stitched {:02}:{:02}:{:02}   •   {} jumps   •   {} transitions   •   q / Esc / Ctrl-C to quit",
        elapsed / 3600,
        elapsed / 60 % 60,
        elapsed % 60,
        dashboard.jump_count,
        dashboard.graph.branch_count(),
    );
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(Color::DarkGray))
            .block(panel("SESSION")),
        area,
    );
}

fn jump_direction(source: usize, destination: usize) -> &'static str {
    if destination < source {
        "backward"
    } else {
        "forward"
    }
}
