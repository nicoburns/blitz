//! Touch event demonstration example
//! 
//! This example demonstrates touch event handling in Blitz.
//! It shows how to handle touchstart, touchmove, touchend, and touchcancel events.

use dioxus::prelude::*;

fn main() {
    dioxus_native::launch(App);
}

#[component]
fn App() -> Element {
    let mut touch_log = use_signal(Vec::<String>::new);
    let mut active_touches = use_signal(Vec::<String>::new);

    rsx! {
        style { {include_str!("touch_demo.css")} }
        
        div {
            class: "container",
            
            h1 { "Touch Event Demo" }
            
            div {
                class: "touch-area",
                ontouchmove: move |evt: Event<TouchData>| {
                    let touch_count = evt.data.touches().len();
                    active_touches.set(vec![format!("Active touches: {}", touch_count)]);
                },
                
                ontouchstart: move |evt: Event<TouchData>| {
                    let msg = format!(
                        "Touch Start: {} touches",
                        evt.data.touches().len()
                    );
                    
                    touch_log.write().push(msg);
                },
                
                ontouchend: move |evt: Event<TouchData>| {
                    let msg = format!(
                        "Touch End: {} touches remaining",
                        evt.data.touches().len()
                    );
                    
                    touch_log.write().push(msg);
                },
                
                ontouchcancel: move |evt: Event<TouchData>| {
                    let msg = format!(
                        "Touch Cancel: {} touches cancelled",
                        evt.data.touches().len()
                    );
                    
                    touch_log.write().push(msg);
                },
                
                div {
                    class: "touch-info",
                    "Touch this area to see events",
                    br {},
                    "Active Touches:",
                    for touch in active_touches() {
                        div { class: "touch-point", "{touch}" }
                    }
                }
            }
            
            div {
                class: "log-container",
                h3 { "Event Log" }
                div {
                    class: "log",
                    for (i, entry) in touch_log().iter().enumerate().rev().take(10) {
                        div { 
                            class: "log-entry",
                            "{i}: {entry}"
                        }
                    }
                }
            }
            
            button {
                onclick: move |_| touch_log.write().clear(),
                "Clear Log"
            }
        }
    }
}
