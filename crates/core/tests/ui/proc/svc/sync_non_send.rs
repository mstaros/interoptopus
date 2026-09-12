use interoptopus::ffi;

#[ffi(service)]
struct Service {
    input: std::rc::Rc<u32>,
}

#[ffi]
impl Service {
    pub fn create() -> Self {
        Self { input: std::rc::Rc::new(1) }
    }

    pub fn get(&self) -> u32 {
        *self.input
    }
}

fn main() {}