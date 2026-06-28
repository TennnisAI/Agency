//! Terminal session daemon: PTY + emulator ownership, IPC protocol, client.

pub mod emulator;

#[cfg(test)]
mod smoke {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    // API correction vs. brief: Processor<T: Timeout> requires explicit type in vte 0.15.
    // Use Processor::<StdSyncHandler>::new() — StdSyncHandler is the default Timeout impl.
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};

    /// Minimal `Dimensions` so we can construct a `Term` headlessly.
    struct Dims { cols: usize, screen: usize, total: usize }
    impl Dimensions for Dims {
        fn total_lines(&self) -> usize { self.total }
        fn screen_lines(&self) -> usize { self.screen }
        fn columns(&self) -> usize { self.cols }
    }

    #[test]
    fn feed_bytes_and_read_a_char() {
        let dims = Dims { cols: 80, screen: 24, total: 24 + 1000 };
        let mut term: Term<VoidListener> = Term::new(Config::default(), &dims, VoidListener);
        let mut parser = Processor::<StdSyncHandler>::new();
        parser.advance(&mut term, b"hi");
        let grid = term.grid();
        let c0 = grid[Line(0)][Column(0)].c;
        let c1 = grid[Line(0)][Column(1)].c;
        assert_eq!((c0, c1), ('h', 'i'));
    }
}
