pub fn greet() {
    println!("{}", inner::banner());
}

mod inner {
    pub fn banner() -> &'static str {
        "hello"
    }
}
