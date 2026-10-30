mod app;
mod ollama;
mod telemetry;

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<app::OllamaApplet>(())
}
