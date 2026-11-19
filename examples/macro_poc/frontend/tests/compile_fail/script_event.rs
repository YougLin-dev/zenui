use zenui_macro_facade::view;

fn main() {
    let _ = view! { <button on:click="alert('no scripts')" /> };
}
