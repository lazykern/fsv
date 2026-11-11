def launch(read_only: bool = False) -> None:
    from fsv.tui.app import FsvApp
    app = FsvApp(read_only=read_only)
    app.run()
