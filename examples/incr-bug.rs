//! Drive the renderer from Dioxus
use dioxus_native::prelude::*;

fn main() {
    dioxus_native::launch(app)
}

pub fn app() -> Element {
    let mut show_box = use_signal(|| false);

    rsx! {
        div {
            style { {CSS} }
            div {
                style: "display: flex;border: 1px solid black;margin: 20px;",
                header {
                    style: "display: inline-flex; align-items: start; min-height: 10px",
                    if show_box() {
                        div {
                            style: "display: block;background: grey;width: 32px;height: 32px;"
                        }
                    }
                    div {
                        style: "display: flex",
                        div { "Wrappable text" }
                    }
                }
            }


            div { style: "display: flex;padding: 20px; gap: 20px;",
                button {
                    class: "counter-button btn-green",
                    onclick: move |_| { *show_box.write() = true; },
                    "Show"
                }
                button {
                    class: "counter-button btn-red",
                    onclick: move |_| { *show_box.write() = false; },
                    "Hide"
                }
            }
        }
    }
}

const CSS: &str = r#"

html, body {
    font-family: sans-serif;
    margin: 0;
    padding: 0;
}


"#;
