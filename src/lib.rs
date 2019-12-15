mod utils;

use wasm_bindgen::prelude::*;
use ttk91::{
    symbolic::Program,
    emulator::{Emulator, Memory, TestIo, InputOutput},
};

use serde_json::json;
use std::collections::HashMap;

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
}

#[wasm_bindgen]
pub struct SymbolicProgram {
    program: ttk91::symbolic::Program,
}

#[wasm_bindgen]
pub fn parse(assembly: &str) -> Result<SymbolicProgram, JsValue> {
    ttk91::symbolic::Program::parse(assembly)
        .map(|program| SymbolicProgram { program })
        .map_err(|err| {
            let err = err.verbose(assembly);
            JsValue::from_serde(&json!({
                "error": err.to_string(),
                "line": err.line,
                "column": err.column,
            })).unwrap()
        })
        //(err.verbose(assembly).line as u32).into())
}

struct QueueIO {
    input: Vec<i32>,
    output: Vec<i32>, 
    calls: Vec<u16>,
}

impl InputOutput for QueueIO {
    fn input(&mut self, _device: u16) -> i32 {
        self.input.remove(0)
    }

    fn output(&mut self, _device: u16, data: i32) {
        self.output.push(data);
    }

    fn supervisor_call(&mut self, code: u16) {
        self.calls.push(code);
    }
}

impl QueueIO {
    fn new() -> QueueIO {
        QueueIO {
            output: Vec::new(),
            input: Vec::new(),
            calls: Vec::new(),
        }
    }
}

#[wasm_bindgen]
pub struct Output {
    output: Vec<i32>,
    calls: Vec<u16>,
    pub line: u32,
}

#[wasm_bindgen]
impl Output {
    pub fn output(&self) -> js_sys::Int32Array {
        unsafe {
            js_sys::Int32Array::view(self.output.as_slice())
        }
    }

    pub fn calls(&self) -> js_sys::Uint16Array {
        unsafe {
            js_sys::Uint16Array::view(self.calls.as_slice())
        }
    }
}

#[wasm_bindgen]
pub struct WasmEmulator {
    emulator: Emulator<Vec<i32>, QueueIO>,
    source_map: HashMap<u16, usize>,
}

#[wasm_bindgen]
impl WasmEmulator {
    pub fn registers(&self) -> Vec<i32> {
        self.emulator.context.r.to_vec()
    }

    pub fn step(&mut self) -> Output {
        self.emulator.step().unwrap();

        let output = self.emulator.io.output.clone();
        let calls = self.emulator.io.calls.clone();

        let line = self.source_map.get(&(self.emulator.context.pc)).unwrap_or(&0);

        Output {
            output,
            calls,
            line: *line as u32,
        }
    }

    pub fn stack_pointer(&self) -> u16 {
        self.emulator.context.r[7] as u16
    }

    pub fn read_addr(&mut self, addr: u16) -> i32 {
        self.emulator.memory.get_data(addr).unwrap()
    }
}

#[wasm_bindgen]
pub fn create_emulator(asm: &str) -> WasmEmulator {
    let program = Program::parse(asm).unwrap();
    let result = program.compile_sourcemap();
    let memory = result.compiled.to_words();

    let emulator = Emulator::new(memory, QueueIO::new())
        .unwrap();

    WasmEmulator {
        emulator,
        source_map: result.source_map,
    }
}

#[wasm_bindgen]
pub fn execute(asm: &str) -> Vec<i32> {
    let program = Program::parse(asm).unwrap();
    let compiled = program.compile();
    let memory = compiled.to_words();

    let mut io = TestIo::new();

    let mut emulator = Emulator::new(memory, &mut io).unwrap();
    emulator.run().unwrap();

    io.into_output()
}

