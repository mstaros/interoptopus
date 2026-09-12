use interoptopus::ffi;

#[ffi(service)]
struct Service {
    input: std::cell::Cell<u32>,
}

#[ffi]
impl Service {
    pub fn create() -> Self {
        Self { input: std::cell::Cell::new(1) }
    }

    pub fn get(&self) -> u32 {
        self.input.get()
    }
}

fn main() {}