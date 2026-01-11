//! Terminal User Interface for interactive agent monitoring.
//! Retro terminal style - green/red on black like classic computers.

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, Clear, ClearType},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Sparkline, Table, Tabs, Clear as ClearWidget},
    Frame, Terminal,
};

use crate::models::{EventType, Session, SessionEvent, SessionStatus};
use crate::storage::Storage;

// Retro Terminal Color Palette - Classic Green on Black
const TERM_GREEN: Color = Color::Rgb(0, 255, 65);        // Bright phosphor green
const TERM_GREEN_DIM: Color = Color::Rgb(0, 180, 45);    // Dimmer green
const TERM_GREEN_DARK: Color = Color::Rgb(0, 100, 25);   // Dark green for backgrounds
const TERM_RED: Color = Color::Rgb(255, 50, 50);         // Alert red
const TERM_AMBER: Color = Color::Rgb(255, 176, 0);       // Amber for warnings
const TERM_BLACK: Color = Color::Rgb(0, 0, 0);           // Pure black background
const TERM_DARK: Color = Color::Rgb(8, 8, 8);            // Slightly lighter black

/// App state for the TUI
pub struct App {
    storage: Storage,
    sessions: Vec<Session>,
    selected_index: usize,
    session_scroll_offset: usize,  // For scrolling sessions list
    tab_index: usize,
    tick_count: u64,
    sparkline_data: Vec<u64>,
    should_quit: bool,
    last_update: Instant,
    animation_frame: usize,
    // Detail view state
    show_detail_view: bool,
    session_events: Vec<SessionEvent>,
    event_scroll_offset: usize,
    selected_event_index: usize,
    event_horizontal_scroll: usize,
    expanded_event_index: Option<usize>,
    expanded_vertical_scroll: usize,  // Vertical scroll within expanded event
    expanded_content_lines: usize,    // Total lines in expanded content
}

impl App {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            sessions: Vec::new(),
            selected_index: 0,
            session_scroll_offset: 0,
            tab_index: 0,
            tick_count: 0,
            sparkline_data: vec![0; 60],
            should_quit: false,
            last_update: Instant::now(),
            animation_frame: 0,
            show_detail_view: false,
            session_events: Vec::new(),
            event_scroll_offset: 0,
            selected_event_index: 0,
            event_horizontal_scroll: 0,
            expanded_event_index: None,
            expanded_vertical_scroll: 0,
            expanded_content_lines: 0,
        }
    }

    /// Toggle detail view and load session events
    pub async fn toggle_detail_view(&mut self) -> Result<()> {
        if self.show_detail_view {
            // Close detail view
            self.show_detail_view = false;
            self.session_events.clear();
            self.event_scroll_offset = 0;
            self.selected_event_index = 0;
            self.event_horizontal_scroll = 0;
            self.expanded_event_index = None;
        } else {
            // Open detail view - load events for selected session
            if !self.sessions.is_empty() && self.selected_index < self.sessions.len() {
                let session_id = &self.sessions[self.selected_index].id;
                self.session_events = self.storage.get_session_events(session_id, 200).await?;
                self.event_scroll_offset = 0;
                self.selected_event_index = 0;
                self.event_horizontal_scroll = 0;
                self.expanded_event_index = None;
                self.show_detail_view = true;
            }
        }
        Ok(())
    }

    /// Move selection up in detail view
    pub fn select_previous_event(&mut self) {
        if self.selected_event_index > 0 {
            self.selected_event_index -= 1;
            self.event_horizontal_scroll = 0; // Reset horizontal scroll on selection change
            // Scroll view if needed
            if self.selected_event_index < self.event_scroll_offset {
                self.event_scroll_offset = self.selected_event_index;
            }
        }
    }

    /// Move selection down in detail view
    pub fn select_next_event(&mut self) {
        if !self.session_events.is_empty() && self.selected_event_index < self.session_events.len() - 1 {
            self.selected_event_index += 1;
            self.event_horizontal_scroll = 0; // Reset horizontal scroll on selection change
            // Scroll view if needed (keep 2 lines margin at bottom)
            let visible_height = 15; // approximate visible rows
            if self.selected_event_index >= self.event_scroll_offset + visible_height {
                self.event_scroll_offset = self.selected_event_index - visible_height + 1;
            }
        }
    }

    /// Scroll text left (show earlier content)
    pub fn scroll_event_left(&mut self) {
        if self.event_horizontal_scroll > 0 {
            self.event_horizontal_scroll = self.event_horizontal_scroll.saturating_sub(20);
        }
    }

    /// Scroll text right (show later content)
    pub fn scroll_event_right(&mut self) {
        self.event_horizontal_scroll += 20;
    }

    /// Toggle expansion of selected event
    pub fn toggle_event_expansion(&mut self) {
        if self.expanded_event_index == Some(self.selected_event_index) {
            self.expanded_event_index = None;
            self.expanded_vertical_scroll = 0;
        } else {
            self.expanded_event_index = Some(self.selected_event_index);
            self.expanded_vertical_scroll = 0;
            // Calculate content lines for the expanded event
            if let Some(event) = self.session_events.get(self.selected_event_index) {
                self.expanded_content_lines = event.content
                    .as_ref()
                    .map(|c| c.lines().count())
                    .unwrap_or(0);
            }
        }
    }

    /// Scroll up within expanded event, or move to previous event if at top
    pub fn scroll_expanded_up(&mut self, visible_lines: usize) {
        if self.expanded_vertical_scroll > 0 {
            // Still have content above, scroll up
            self.expanded_vertical_scroll = self.expanded_vertical_scroll.saturating_sub(1);
        } else {
            // At top of content - move to previous event
            if self.selected_event_index > 0 {
                self.selected_event_index -= 1;
                self.expanded_event_index = Some(self.selected_event_index);
                // Calculate new content lines and scroll to bottom
                if let Some(event) = self.session_events.get(self.selected_event_index) {
                    self.expanded_content_lines = event.content
                        .as_ref()
                        .map(|c| c.lines().count())
                        .unwrap_or(0);
                    // Start at bottom of previous event
                    self.expanded_vertical_scroll = self.expanded_content_lines.saturating_sub(visible_lines);
                }
                // Adjust scroll offset if needed
                if self.selected_event_index < self.event_scroll_offset {
                    self.event_scroll_offset = self.selected_event_index;
                }
            }
        }
    }

    /// Scroll down within expanded event, or move to next event if at bottom
    pub fn scroll_expanded_down(&mut self, visible_lines: usize) {
        let max_scroll = self.expanded_content_lines.saturating_sub(visible_lines);
        if self.expanded_vertical_scroll < max_scroll {
            // Still have content below, scroll down
            self.expanded_vertical_scroll += 1;
        } else {
            // At bottom of content - move to next event
            if self.selected_event_index < self.session_events.len().saturating_sub(1) {
                self.selected_event_index += 1;
                self.expanded_event_index = Some(self.selected_event_index);
                self.expanded_vertical_scroll = 0;
                // Calculate new content lines
                if let Some(event) = self.session_events.get(self.selected_event_index) {
                    self.expanded_content_lines = event.content
                        .as_ref()
                        .map(|c| c.lines().count())
                        .unwrap_or(0);
                }
            }
        }
    }

    pub async fn refresh_data(&mut self) -> Result<()> {
        // Remember currently selected session ID to preserve selection
        let selected_session_id = self.sessions
            .get(self.selected_index)
            .map(|s| s.id.clone());

        self.sessions = self.storage.get_active_sessions(50).await?;

        // Update sparkline with active session count
        self.sparkline_data.remove(0);
        self.sparkline_data.push(self.sessions.len() as u64);

        // Try to find the previously selected session in the new list
        if let Some(ref old_id) = selected_session_id {
            if let Some(new_idx) = self.sessions.iter().position(|s| &s.id == old_id) {
                self.selected_index = new_idx;
            }
        }

        // Keep selected_index in bounds
        if !self.sessions.is_empty() && self.selected_index >= self.sessions.len() {
            self.selected_index = self.sessions.len() - 1;
        }

        self.last_update = Instant::now();
        Ok(())
    }

    /// Refresh events for current session (live updates in detail view)
    /// Events are newest-first (ORDER BY DESC), so new events appear at top (index 0)
    /// Preserves user's current selection by tracking event ID.
    pub async fn refresh_events(&mut self) -> Result<()> {
        if !self.sessions.is_empty() && self.selected_index < self.sessions.len() {
            let session_id = &self.sessions[self.selected_index].id;

            // Remember currently selected event ID to preserve selection
            let selected_event_id = self.session_events
                .get(self.selected_event_index)
                .map(|e| e.id.clone());

            let old_count = self.session_events.len();
            self.session_events = self.storage.get_session_events(session_id, 500).await?;
            let new_count = self.session_events.len();

            // Try to find the previously selected event in the new list
            if let Some(ref old_id) = selected_event_id {
                if let Some(new_idx) = self.session_events.iter().position(|e| &e.id == old_id) {
                    // Found it - adjust selection to new position
                    self.selected_event_index = new_idx;
                    // Adjust scroll to keep selection visible
                    if self.selected_event_index < self.event_scroll_offset {
                        self.event_scroll_offset = self.selected_event_index;
                    }
                } else if new_count > old_count {
                    // Event not found but new events added - shift selection down
                    let added = new_count - old_count;
                    self.selected_event_index = self.selected_event_index.saturating_add(added);
                    self.event_scroll_offset = self.event_scroll_offset.saturating_add(added);
                }
            }

            // Bounds check
            if self.selected_event_index >= self.session_events.len() {
                self.selected_event_index = self.session_events.len().saturating_sub(1);
            }
        }
        Ok(())
    }

    pub fn next_session(&mut self) {
        if !self.sessions.is_empty() {
            if self.selected_index < self.sessions.len() - 1 {
                self.selected_index += 1;
                // Scroll down if selection goes below visible area (keep 2 rows margin)
                let visible_rows = 10; // approximate visible rows in table
                if self.selected_index >= self.session_scroll_offset + visible_rows {
                    self.session_scroll_offset = self.selected_index - visible_rows + 1;
                }
            }
        }
    }

    pub fn previous_session(&mut self) {
        if !self.sessions.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
            // Scroll up if selection goes above visible area
            if self.selected_index < self.session_scroll_offset {
                self.session_scroll_offset = self.selected_index;
            }
        }
    }

    pub fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % 3;
    }

    pub fn previous_tab(&mut self) {
        self.tab_index = if self.tab_index == 0 { 2 } else { self.tab_index - 1 };
    }

    pub fn tick(&mut self) {
        self.tick_count += 1;
        self.animation_frame = (self.animation_frame + 1) % 8;
    }
}

/// Run the interactive TUI
pub async fn run_tui(storage: Storage) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Clear(ClearType::All))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(storage);
    app.refresh_data().await?;

    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if app.show_detail_view {
                    // Detail view controls - check if in expanded mode first
                    if app.expanded_event_index.is_some() {
                        // Expanded event view controls
                        // Approximate visible lines (terminal height - chrome)
                        let visible_lines = terminal.size().map(|s| s.height.saturating_sub(8) as usize).unwrap_or(20);
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                // Just collapse the expanded view, don't close detail view
                                app.expanded_event_index = None;
                                app.expanded_vertical_scroll = 0;
                                app.event_horizontal_scroll = 0;
                            }
                            KeyCode::Enter => {
                                // Collapse and stay on current event
                                app.expanded_event_index = None;
                                app.expanded_vertical_scroll = 0;
                            }
                            KeyCode::Up | KeyCode::Char('k') => app.scroll_expanded_up(visible_lines),
                            KeyCode::Down | KeyCode::Char('j') => app.scroll_expanded_down(visible_lines),
                            KeyCode::Left | KeyCode::Char('h') => app.scroll_event_left(),
                            KeyCode::Right | KeyCode::Char('l') => app.scroll_event_right(),
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.should_quit = true
                            }
                            _ => {}
                        }
                    } else {
                        // Events list view controls
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                // Close detail view and go back to sessions
                                app.show_detail_view = false;
                                app.session_events.clear();
                                app.selected_event_index = 0;
                                app.event_scroll_offset = 0;
                            }
                            KeyCode::Enter => app.toggle_event_expansion(),
                            KeyCode::Down | KeyCode::Char('j') => app.select_next_event(),
                            KeyCode::Up | KeyCode::Char('k') => app.select_previous_event(),
                            KeyCode::Left | KeyCode::Char('h') => app.scroll_event_left(),
                            KeyCode::Right | KeyCode::Char('l') => app.scroll_event_right(),
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.should_quit = true
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Main view controls
                    match key.code {
                        KeyCode::Char('q') => app.should_quit = true,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true
                        }
                        KeyCode::Down | KeyCode::Char('j') => app.next_session(),
                        KeyCode::Up | KeyCode::Char('k') => app.previous_session(),
                        KeyCode::Tab => app.next_tab(),
                        KeyCode::BackTab => app.previous_tab(),
                        KeyCode::Enter => {
                            app.toggle_detail_view().await?;
                        }
                        KeyCode::Char('r') => {
                            app.refresh_data().await?;
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick();

            // Refresh data every 2 seconds
            if app.tick_count % 20 == 0 {
                app.refresh_data().await?;
            }

            // Refresh events every 1 second when in detail view (live updates)
            // BUT pause refresh when user has an event expanded (reading)
            if app.show_detail_view && app.expanded_event_index.is_none() && app.tick_count % 10 == 0 {
                app.refresh_events().await?;
            }

            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Clear entire screen with black background first
    f.render_widget(ClearWidget, size);
    f.render_widget(
        Block::default().style(Style::default().bg(TERM_BLACK)),
        size
    );

    // Show detail view if active
    if app.show_detail_view {
        render_full_detail_view(f, size, app);
        return;
    }

    // Create main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Tabs
            Constraint::Min(10),    // Main content
            Constraint::Length(3),  // Footer/help
        ])
        .split(size);

    // Render header with animation
    render_header(f, chunks[0], app);

    // Render tabs
    render_tabs(f, chunks[1], app);

    // Render main content based on selected tab
    match app.tab_index {
        0 => render_sessions_tab(f, chunks[2], app),
        1 => render_details_tab(f, chunks[2], app),
        2 => render_metrics_tab(f, chunks[2], app),
        _ => {}
    }

    // Render footer
    render_footer(f, chunks[3], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    // Retro blinking cursor effect
    let cursor = if app.animation_frame % 2 == 0 { "█" } else { " " };
    let scan_line = match app.animation_frame % 4 {
        0 => "▁",
        1 => "▂",
        2 => "▃",
        _ => "▄",
    };

    let title = format!(
        " {} AGENT MONITOR v0.1.0 {} Active: {} {}",
        scan_line, cursor, app.sessions.len(), scan_line
    );

    let header = Paragraph::new(title)
        .style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK)),
        );
    f.render_widget(header, area);
}

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles = vec!["[1] SESSIONS", "[2] DETAILS", "[3] METRICS"];
    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
        )
        .select(app.tab_index)
        .style(Style::default().fg(TERM_GREEN_DIM).bg(TERM_BLACK))
        .highlight_style(
            Style::default()
                .fg(TERM_BLACK)
                .bg(TERM_GREEN)
                .add_modifier(Modifier::BOLD),
        )
        .divider(symbols::line::VERTICAL);
    f.render_widget(tabs, area);
}

fn render_sessions_tab(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(area);

    // Sessions table with selector indicator and scrolling
    let header_cells = [" ", "AGENT", "PROJECT", "STATUS", "MSGS", "TOKENS", "COST"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells)
        .height(1)
        .bottom_margin(0)
        .style(Style::default().bg(TERM_BLACK));

    // Calculate visible rows and apply scroll offset
    let visible_rows = (chunks[0].height as usize).saturating_sub(4); // Account for borders and header
    let rows: Vec<Row> = app.sessions
        .iter()
        .enumerate()
        .skip(app.session_scroll_offset)
        .take(visible_rows)
        .map(|(i, session)| {
            let is_selected = i == app.selected_index;

            let (fg, bg, selector) = if is_selected {
                (TERM_BLACK, TERM_GREEN, "▶")  // Inverted colors + arrow for selection
            } else {
                (TERM_GREEN, TERM_BLACK, " ")
            };

            let project_name = session.project_path.split('/').last().unwrap_or("---");
            let status_display = match session.status {
                SessionStatus::Active => "[LIVE]",
                SessionStatus::Idle => "[IDLE]",
                SessionStatus::Completed => "[DONE]",
                SessionStatus::Crashed => "[ERR!]",
                SessionStatus::Unknown => "[????]",
            };
            let tokens = format_tokens(session.tokens_input + session.tokens_output);
            let cost = format!("${:.2}", session.estimated_cost);

            Row::new(vec![
                Cell::from(selector).style(Style::default().fg(TERM_GREEN).bg(bg).add_modifier(Modifier::BOLD)),
                Cell::from(format!("{:<10}", truncate_str(&session.agent_type.to_string(), 10))),
                Cell::from(truncate_str(project_name, 12)),
                Cell::from(status_display),
                Cell::from(format!("{:>4}", session.message_count)),
                Cell::from(format!("{:>6}", tokens)),
                Cell::from(format!("{:>6}", cost)),
            ])
            .style(Style::default().fg(fg).bg(bg))
            .height(1)
        }).collect();

    // Update title to show scroll position
    let title = if app.sessions.len() > visible_rows {
        format!(" SESSIONS [{}-{}/{}] ",
            app.session_scroll_offset + 1,
            (app.session_scroll_offset + visible_rows).min(app.sessions.len()),
            app.sessions.len()
        )
    } else {
        format!(" ACTIVE SESSIONS ({}) ", app.sessions.len())
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),   // Selector
            Constraint::Length(11),  // Agent
            Constraint::Length(13),  // Project
            Constraint::Length(7),   // Status
            Constraint::Length(5),   // Msgs
            Constraint::Length(7),   // Tokens
            Constraint::Length(7),   // Cost
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(TERM_GREEN_DIM))
            .style(Style::default().bg(TERM_BLACK))
            .title(title)
            .title_style(Style::default().fg(TERM_GREEN).add_modifier(Modifier::BOLD)),
    )
    .style(Style::default().bg(TERM_BLACK));

    f.render_widget(table, chunks[0]);

    // Activity sparkline and summary
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(4)])
        .split(chunks[1]);

    // Sparkline
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(" ACTIVITY ")
                .title_style(Style::default().fg(TERM_GREEN)),
        )
        .data(&app.sparkline_data)
        .style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK));
    f.render_widget(sparkline, right_chunks[0]);

    // Summary stats
    let total_tokens: i64 = app.sessions.iter().map(|s| s.tokens_input + s.tokens_output).sum();
    let total_cost: f64 = app.sessions.iter().map(|s| s.estimated_cost).sum();
    let total_messages: i64 = app.sessions.iter().map(|s| s.message_count).sum();

    let summary_text = vec![
        Line::from(Span::styled(
            format!("TOKENS: {}", format_tokens(total_tokens)),
            Style::default().fg(TERM_GREEN)
        )),
        Line::from(Span::styled(
            format!("COST:   ${:.2}", total_cost),
            Style::default().fg(TERM_AMBER)
        )),
        Line::from(Span::styled(
            format!("MSGS:   {}", total_messages),
            Style::default().fg(TERM_GREEN)
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("UPD: {}s ago", app.last_update.elapsed().as_secs()),
            Style::default().fg(TERM_GREEN_DIM)
        )),
    ];

    let summary = Paragraph::new(summary_text)
        .style(Style::default().bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(" TOTALS ")
                .title_style(Style::default().fg(TERM_GREEN)),
        );
    f.render_widget(summary, right_chunks[1]);
}

fn render_details_tab(f: &mut Frame, area: Rect, app: &App) {
    if app.sessions.is_empty() || app.selected_index >= app.sessions.len() {
        let empty = Paragraph::new("NO SESSION SELECTED - USE ARROW KEYS IN SESSIONS TAB")
            .style(Style::default().fg(TERM_GREEN_DIM).bg(TERM_BLACK))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(TERM_GREEN_DIM))
                    .style(Style::default().bg(TERM_BLACK))
                    .title(" SESSION DETAILS "),
            );
        f.render_widget(empty, area);
        return;
    }

    let session = &app.sessions[app.selected_index];

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Session info
    let project_name = session.project_path.split('/').last().unwrap_or("UNKNOWN");
    let details = vec![
        Line::from(vec![
            Span::styled("PROJECT: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(project_name, Style::default().fg(TERM_GREEN).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("PATH: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(&session.project_path, Style::default().fg(TERM_GREEN)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("AGENT: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(session.agent_type.to_string(), Style::default().fg(TERM_GREEN)),
        ]),
        Line::from(vec![
            Span::styled("MODEL: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(
                session.model_id.as_deref().unwrap_or("UNKNOWN"),
                Style::default().fg(TERM_GREEN),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("STATUS: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(
                format!("{:?}", session.status).to_uppercase(),
                Style::default().fg(match session.status {
                    SessionStatus::Active => TERM_GREEN,
                    SessionStatus::Idle => TERM_AMBER,
                    SessionStatus::Completed => TERM_GREEN,
                    SessionStatus::Crashed => TERM_RED,
                    SessionStatus::Unknown => TERM_GREEN_DIM,
                }).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("ID: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(&session.id[..16.min(session.id.len())], Style::default().fg(TERM_GREEN_DIM)),
        ]),
        Line::from(vec![
            Span::styled("STARTED: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(
                session.started_at.format("%H:%M:%S").to_string(),
                Style::default().fg(TERM_GREEN),
            ),
        ]),
        Line::from(vec![
            Span::styled("DURATION: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(
                format_duration(session.duration_seconds),
                Style::default().fg(TERM_GREEN),
            ),
        ]),
    ];

    let details_widget = Paragraph::new(details)
        .style(Style::default().bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(format!(" {} ", project_name.to_uppercase()))
                .title_style(Style::default().fg(TERM_GREEN).add_modifier(Modifier::BOLD)),
        );
    f.render_widget(details_widget, chunks[0]);

    // Token usage breakdown
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(4)])
        .split(chunks[1]);

    let total_tokens = session.tokens_input + session.tokens_output;
    let input_ratio = if total_tokens > 0 {
        (session.tokens_input as f64 / total_tokens as f64 * 100.0) as u16
    } else {
        50
    };

    let token_info = vec![
        Line::from(vec![
            Span::styled("INPUT:  ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(format_tokens(session.tokens_input), Style::default().fg(TERM_GREEN)),
        ]),
        Line::from(vec![
            Span::styled("OUTPUT: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(format_tokens(session.tokens_output), Style::default().fg(TERM_GREEN)),
        ]),
        Line::from(vec![
            Span::styled("TOTAL:  ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(format_tokens(total_tokens), Style::default().fg(TERM_GREEN).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("COST: ", Style::default().fg(TERM_GREEN_DIM)),
            Span::styled(format!("${:.4}", session.estimated_cost), Style::default().fg(TERM_AMBER)),
        ]),
    ];

    let tokens_widget = Paragraph::new(token_info)
        .style(Style::default().bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(" TOKEN USAGE ")
                .title_style(Style::default().fg(TERM_GREEN)),
        );
    f.render_widget(tokens_widget, right_chunks[0]);

    // Token ratio gauge
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(" I/O RATIO ")
                .title_style(Style::default().fg(TERM_GREEN)),
        )
        .gauge_style(Style::default().fg(TERM_GREEN).bg(TERM_DARK))
        .percent(input_ratio)
        .label(Span::styled(
            format!("{}% IN / {}% OUT", input_ratio, 100 - input_ratio),
            Style::default().fg(TERM_GREEN).add_modifier(Modifier::BOLD)
        ));
    f.render_widget(gauge, right_chunks[1]);
}

fn render_metrics_tab(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Agent type distribution - use BTreeMap for stable ordering
    let mut agent_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for session in &app.sessions {
        *agent_counts.entry(session.agent_type.to_string()).or_insert(0) += 1;
    }

    let items: Vec<ListItem> = agent_counts
        .iter()
        .map(|(agent, count)| {
            let bar_len = (*count as f64 / app.sessions.len().max(1) as f64 * 20.0) as usize;
            let bar = "█".repeat(bar_len) + &"░".repeat(20 - bar_len);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:12}", agent.to_uppercase()), Style::default().fg(TERM_GREEN)),
                Span::styled(bar, Style::default().fg(TERM_GREEN)),
                Span::styled(format!(" {}", count), Style::default().fg(TERM_GREEN).add_modifier(Modifier::BOLD)),
            ]))
        })
        .collect();

    let agent_list = List::new(items)
        .style(Style::default().bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(" AGENT DISTRIBUTION ")
                .title_style(Style::default().fg(TERM_GREEN)),
        );
    f.render_widget(agent_list, chunks[0]);

    // Cost and token breakdown - use BTreeMap for stable ordering
    let mut costs_by_agent: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for session in &app.sessions {
        *costs_by_agent.entry(session.agent_type.to_string()).or_insert(0.0) += session.estimated_cost;
    }

    let cost_items: Vec<ListItem> = costs_by_agent
        .iter()
        .map(|(agent, cost)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:12}", agent.to_uppercase()), Style::default().fg(TERM_GREEN)),
                Span::styled(format!("${:.4}", cost), Style::default().fg(TERM_AMBER)),
            ]))
        })
        .collect();

    let cost_list = List::new(cost_items)
        .style(Style::default().bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(" COST BY AGENT ")
                .title_style(Style::default().fg(TERM_GREEN)),
        );
    f.render_widget(cost_list, chunks[1]);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let blink = if app.animation_frame % 4 < 2 { "█" } else { " " };

    let help_text = format!(
        " READY{} | ↑↓/jk:NAV | ENTER:VIEW | TAB:SWITCH | r:REFRESH | q:QUIT ",
        blink
    );

    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DARK))
                .style(Style::default().bg(TERM_BLACK)),
        );
    f.render_widget(footer, area);
}

/// Render full detail view showing session conversation and events
fn render_full_detail_view(f: &mut Frame, area: Rect, app: &App) {
    // Check if we're showing an expanded event
    if let Some(expanded_idx) = app.expanded_event_index {
        render_expanded_event(f, area, app, expanded_idx);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(10),    // Events/conversation
            Constraint::Length(3),  // Footer
        ])
        .split(area);

    // Get session info if available
    let session = if !app.sessions.is_empty() && app.selected_index < app.sessions.len() {
        Some(&app.sessions[app.selected_index])
    } else {
        None
    };

    // Header with session info
    let title = if let Some(s) = session {
        let project_name = s.project_path.split('/').last().unwrap_or("UNKNOWN");
        format!(
            " {} | {} | {} msgs | ${:.4} ",
            project_name.to_uppercase(),
            s.agent_type.to_string().to_uppercase(),
            s.message_count,
            s.estimated_cost
        )
    } else {
        " NO SESSION ".to_string()
    };

    let header = Paragraph::new(title)
        .style(Style::default().fg(TERM_BLACK).bg(TERM_GREEN).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN))
                .style(Style::default().bg(TERM_GREEN)),
        );
    f.render_widget(header, chunks[0]);

    // Events/conversation list with selection
    let visible_count = (chunks[1].height as usize).saturating_sub(2);
    let content_width = (area.width as usize).saturating_sub(4);
    let h_scroll = app.event_horizontal_scroll;

    let items: Vec<ListItem> = app.session_events
        .iter()
        .enumerate()
        .skip(app.event_scroll_offset)
        .take(visible_count)
        .map(|(idx, event)| {
            let is_selected = idx == app.selected_event_index;

            let (icon, color) = match event.event_type {
                EventType::PromptReceived => ("→ USER  ", TERM_AMBER),
                EventType::ResponseGenerated => ("← AGENT ", TERM_GREEN),
                EventType::Thinking => ("◊ THINK ", Color::Rgb(150, 150, 255)),
                EventType::ToolStart => ("▶ TOOL  ", Color::Rgb(100, 200, 255)),
                EventType::ToolComplete | EventType::ToolExecuted => ("◀ DONE  ", Color::Rgb(100, 200, 255)),
                EventType::FileRead => ("◉ READ  ", Color::Rgb(255, 200, 100)),
                EventType::FileModified => ("◉ WRITE ", Color::Rgb(255, 150, 100)),
                EventType::Error => ("✗ ERR   ", TERM_RED),
                EventType::SessionStart => ("● START ", TERM_GREEN),
                EventType::SessionEnd => ("○ END   ", TERM_GREEN_DIM),
                EventType::Custom => ("? MISC  ", TERM_GREEN_DIM),
            };

            let time = event.timestamp.format("%H:%M:%S").to_string();

            // Get full content
            let content = event.content.as_deref()
                .or(event.tool_name.as_deref())
                .or(event.file_path.as_deref())
                .unwrap_or("(no content)");

            // Apply horizontal scroll only to selected item
            let content_display = if is_selected && h_scroll > 0 {
                if h_scroll < content.len() {
                    &content[h_scroll..]
                } else {
                    "(end of content)"
                }
            } else {
                content
            };

            // Truncate for display (but show ... to indicate more)
            let max_width = content_width.saturating_sub(20);
            let display_text = if content_display.len() > max_width {
                format!("{}→", &content_display[..max_width.saturating_sub(1)])
            } else {
                content_display.to_string()
            };

            // Selection indicator
            let selector = if is_selected { "▶" } else { " " };

            // Style based on selection
            let (fg, bg) = if is_selected {
                (TERM_BLACK, color)
            } else {
                (TERM_GREEN, TERM_BLACK)
            };

            ListItem::new(Line::from(vec![
                Span::styled(selector, Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} ", time), Style::default().fg(if is_selected { TERM_BLACK } else { TERM_GREEN_DIM }).bg(bg)),
                Span::styled(icon, Style::default().fg(if is_selected { TERM_BLACK } else { color }).bg(bg).add_modifier(Modifier::BOLD)),
                Span::styled(display_text, Style::default().fg(fg).bg(bg)),
            ]))
        }).collect();

    let scroll_info = format!(
        " EVENTS [{}/{}] h-scroll:{} ",
        app.selected_event_index + 1,
        app.session_events.len(),
        app.event_horizontal_scroll
    );

    let events_list = List::new(items)
        .style(Style::default().bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(scroll_info)
                .title_style(Style::default().fg(TERM_GREEN)),
        );
    f.render_widget(events_list, chunks[1]);

    // Footer with controls
    let footer_text = " ↑↓:SELECT | ←→:SCROLL | ENTER:EXPAND | ESC/q:CLOSE ";
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DARK))
                .style(Style::default().bg(TERM_BLACK)),
        );
    f.render_widget(footer, chunks[2]);
}

/// Render an expanded event showing full content
fn render_expanded_event(f: &mut Frame, area: Rect, app: &App, event_idx: usize) {
    let event = match app.session_events.get(event_idx) {
        Some(e) => e,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(10),    // Content
            Constraint::Length(3),  // Footer
        ])
        .split(area);

    // Header with event info
    let (icon, color) = match event.event_type {
        EventType::PromptReceived => ("USER MESSAGE", TERM_AMBER),
        EventType::ResponseGenerated => ("AGENT RESPONSE", TERM_GREEN),
        EventType::Thinking => ("THINKING", Color::Rgb(150, 150, 255)),
        EventType::ToolStart => ("TOOL CALL", Color::Rgb(100, 200, 255)),
        EventType::ToolComplete | EventType::ToolExecuted => ("TOOL RESULT", Color::Rgb(100, 200, 255)),
        EventType::FileRead => ("FILE READ", Color::Rgb(255, 200, 100)),
        EventType::FileModified => ("FILE WRITE", Color::Rgb(255, 150, 100)),
        EventType::Error => ("ERROR", TERM_RED),
        _ => ("EVENT", TERM_GREEN_DIM),
    };

    let time = event.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
    let title = format!(" {} | {} ", icon, time);

    let header = Paragraph::new(title)
        .style(Style::default().fg(TERM_BLACK).bg(color).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .style(Style::default().bg(color)),
        );
    f.render_widget(header, chunks[0]);

    // Full content with word wrap
    let content = event.content.as_deref()
        .or(event.tool_name.as_deref())
        .or(event.file_path.as_deref())
        .unwrap_or("(no content)");

    let total_lines = content.lines().count();
    let visible_lines = chunks[1].height.saturating_sub(2) as usize;
    let v_scroll = app.expanded_vertical_scroll;
    let h_scroll = app.event_horizontal_scroll;

    // Apply horizontal scroll to each line
    let display_content: String = if h_scroll > 0 {
        content.lines()
            .map(|line| {
                if h_scroll < line.len() {
                    &line[h_scroll..]
                } else {
                    ""
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content.to_string()
    };

    let content_para = Paragraph::new(display_content)
        .style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((v_scroll as u16, 0))  // Apply vertical scroll
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DIM))
                .style(Style::default().bg(TERM_BLACK))
                .title(format!(" LINE {}/{} | {} chars | h:{} ",
                    v_scroll + 1, total_lines, content.len(), h_scroll))
                .title_style(Style::default().fg(TERM_GREEN)),
        );
    f.render_widget(content_para, chunks[1]);

    // Footer with navigation hint
    let at_top = v_scroll == 0;
    let at_bottom = v_scroll >= total_lines.saturating_sub(visible_lines);
    let nav_hint = if at_top && at_bottom {
        "↑↓:NAV EVENTS"
    } else if at_top {
        "↑:PREV EVENT | ↓:SCROLL"
    } else if at_bottom {
        "↑:SCROLL | ↓:NEXT EVENT"
    } else {
        "↑↓:SCROLL"
    };
    let footer_text = format!(" {} | ←→:H-SCROLL | ENTER:COLLAPSE | ESC:CLOSE ", nav_hint);
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(TERM_GREEN).bg(TERM_BLACK))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TERM_GREEN_DARK))
                .style(Style::default().bg(TERM_BLACK)),
        );
    f.render_widget(footer, chunks[2]);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}..", &s[..max_len.saturating_sub(2)])
    } else {
        s.to_string()
    }
}

fn format_tokens(count: i64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        format!("{}", count)
    }
}

fn format_duration(seconds: f64) -> String {
    if seconds >= 3600.0 {
        format!("{:.1}h", seconds / 3600.0)
    } else if seconds >= 60.0 {
        format!("{:.1}m", seconds / 60.0)
    } else {
        format!("{:.0}s", seconds)
    }
}
