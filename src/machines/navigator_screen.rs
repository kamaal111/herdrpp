//! Inspect completed native frames; terminal output is still forwarded unchanged.
use anyhow::{Context, Result, bail};

pub struct NavigatorScreen {
    parser: vt100::Parser,
    completed: vt100::Screen,
    pub epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigator {
    pub machine_rows: Vec<(u16, String)>,
    pub ready_machines: Vec<String>,
    pub pane_count: usize,
    pub current_machine: Option<String>,
    pub search: String,
    pub selected_machine: Option<String>,
    pub search_focused: bool,
}

impl Navigator {
    /// Herdr 0.9.5 separates the editor from its right-aligned result count
    /// with at least two spaces. Controller queries are short ASCII aliases.
    pub fn query(&self) -> &str {
        let text = self.search.trim_start().strip_prefix("/ ").unwrap_or("");
        let query = text.split("  ").next().unwrap_or("").trim();
        if !self.search_focused && query == "search agents and terminals" {
            ""
        } else {
            query
        }
    }
}

impl NavigatorScreen {
    pub fn new(rows: u16, columns: u16) -> Self {
        let parser = vt100::Parser::new(rows, columns, 0);
        let completed = parser.screen().clone();
        Self {
            parser,
            completed,
            epoch: 0,
        }
    }

    /// The transport calls this for complete VT tokens, not arbitrary reads.
    pub fn token(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        if bytes == b"\x1b[?2026l" {
            self.completed = self.parser.screen().clone();
            self.epoch += 1;
        }
    }

    pub fn reset_size(&mut self, rows: u16, columns: u16) {
        *self = Self::new(rows, columns);
    }

    pub fn navigator(&self) -> Result<Option<Navigator>> {
        let (height, width) = self.completed.size();
        let panel_width = width.saturating_sub(4).min(116);
        let panel_height = height.saturating_sub(2).min(42);
        if panel_width < 4 || panel_height < 9 {
            return Ok(None);
        }
        let left = usize::from((width - panel_width) / 2);
        let top = usize::from((height - panel_height) / 2);
        let bottom = top + usize::from(panel_height) - 1;
        let cell = |row: usize, column: usize| {
            self.completed
                .cell(row as u16, column as u16)
                .map(vt100::Cell::contents)
        };
        let title = self
            .completed
            .rows(left as u16, panel_width)
            .nth(top)
            .context("native navigator title is absent")?;
        if cell(top, left).as_deref() != Some("┌") || !title.contains("─ Go to ─") {
            return Ok(None);
        }
        if cell(bottom, left).as_deref() != Some("└")
            || cell(top, left + usize::from(panel_width) - 1).as_deref() != Some("┐")
            || cell(bottom, left + usize::from(panel_width) - 1).as_deref() != Some("┘")
        {
            bail!("native navigator frame is incomplete");
        }
        let mut machine_rows = Vec::new();
        let mut ready_machines = Vec::new();
        let mut current_machine = None;
        let mut ancestry = None;
        let mut pane_count = 0;
        let mut selected_machine = None;
        let search = self
            .completed
            .rows((left + 1) as u16, panel_width - 2)
            .nth(top + 1)
            .context("native navigator search row is absent")?;
        for row in top + 3..bottom.saturating_sub(3) {
            let line = self
                .completed
                .rows((left + 1) as u16, panel_width - 2)
                .nth(row)
                .context("native navigator row is absent")?;
            if line.starts_with(' ')
                && !line.starts_with("  ")
                && !line.trim().is_empty()
                && line.trim() != "No matching agents or terminals"
            {
                let heading = line
                    .trim()
                    .split("  ")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                ancestry = Some(heading.clone());
                if heading == "Local" || line.trim_end().ends_with('●') {
                    ready_machines.push(heading.clone());
                }
                if self
                    .completed
                    .cell(row as u16, (left + 1) as u16)
                    .map(|cell| cell.bgcolor())
                    != self
                        .completed
                        .cell((bottom - 1) as u16, (left + 1) as u16)
                        .map(|cell| cell.bgcolor())
                {
                    selected_machine = Some(heading.clone());
                }
                machine_rows.push((row as u16, heading));
            }
            if line.starts_with("   ├─ ") || line.starts_with("   └─ ") {
                pane_count += 1;
            }
            if (line.starts_with("   ├─ ◆ ") || line.starts_with("   └─ ◆ "))
                // Native pane connectors have overlay foreground; a workspace
                // label cannot acquire that per-cell structural styling. Bold
                // is not reliable: Herdr retains it after selection changes.
                && self.completed.cell(row as u16, (left + 4) as u16).map(|cell| cell.fgcolor())
                    != self.completed.cell(row as u16, (left + 7) as u16).map(|cell| cell.fgcolor())
            {
                if current_machine.is_some() {
                    bail!("native navigator has more than one current pane");
                }
                current_machine = ancestry.clone();
            }
        }
        Ok(Some(Navigator {
            machine_rows,
            ready_machines,
            pane_count,
            current_machine,
            search,
            selected_machine,
            search_focused: !self.completed.hide_cursor(),
        }))
    }
}
