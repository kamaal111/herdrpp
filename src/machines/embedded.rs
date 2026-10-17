//! A fullscreen backend that erases by writing cells, preserving the host graphics cache.
use std::io;

use ratatui::{
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};

pub struct Embedded(pub CrosstermBackend<io::Stdout>);
impl Backend for Embedded {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.0.draw(content)
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        self.0.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.0.show_cursor()
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(Position::ORIGIN)
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.0.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        let size = self.size()?;
        let cell = Cell::default();
        for row in 0..size.height {
            self.0
                .draw((0..size.width).map(|column| (column, row, &cell)))?;
        }
        self.0.flush()
    }
    fn clear_region(&mut self, _: ClearType) -> io::Result<()> {
        self.clear()
    }
    fn size(&self) -> io::Result<Size> {
        self.0.size()
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.0.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
