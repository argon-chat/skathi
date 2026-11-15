//! Prints the CSS support catalog as JSON: `cargo run -p sk-css --example css-catalog`.

fn main() {
    println!("{}", sk_css::catalog::to_json());
}
