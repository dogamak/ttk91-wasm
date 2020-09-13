mod utils;

use wasm_bindgen::prelude::*;
use ttk91::{
    parsing::{Context, LineSpan},
    symbolic::{Program, parser::ParseError},
    symbol_table::{Label, Value},
    emulator::{Emulator, BalloonMemory, Memory, TestIo, InputOutput},
    event::{Event, EventListener},
    source_map::SourceMap,
};

use serde_derive::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::rc::Rc;

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
#[derive(Serialize, Debug, Clone, Copy)]
pub enum ParseErrorLevel {
    Suggestion,
    Error,
}

#[wasm_bindgen]
#[derive(Serialize)]
pub struct JsParseError {
    pub level: ParseErrorLevel,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    message: String,
}

#[wasm_bindgen]
impl JsParseError {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }
}

#[wasm_bindgen]
pub struct SymbolicProgram {
    program: Program,
}

fn calculate_position(input: &str, offset: usize) -> (usize, usize) {
    input[..offset]
        .split('\n')
        .fold((0,0), |(l,_), line| (l+1, line.len()))
}

fn into_js_errors(input: &str, error: ParseError) -> Vec<JsParseError> {
    let offset = error.span()
        .map(|span| span.start)
        .unwrap_or(input.len());

    let (start_line, start_column, end_line, end_column) = match error.span() {
        Some(span) => {
            let (sl, sc) = calculate_position(input, span.start);
            let (el, ec) = calculate_position(input, span.end);
            (sl, sc, el, ec)
        },
        None => (0, 0, 0, 0),
    };

    let mut results = Vec::new();

    results.push(JsParseError {
        level: ParseErrorLevel::Error,
        start_line,
        start_column,
        end_line,
        end_column,
        message: error.to_string(),
    });

    for ctx in error.get_context() {
        if let Context::Suggestion { span, message } = ctx {
            let (start_line, start_column) = calculate_position(input, span.start);
            let (end_line, end_column) = calculate_position(input, span.end);

            results.push(JsParseError {
                level: ParseErrorLevel::Suggestion,
                start_line,
                start_column,
                end_line,
                end_column,
                message: message.to_string(),
            });
        }
    }

    results
}

#[wasm_bindgen]
pub fn parse(input: &str) -> Result<SymbolicProgram, JsValue> {
    Program::parse_verbose(input)
        .map(|program| SymbolicProgram { program })
        .map_err(|errors| {
            let errors = errors.into_iter()
                .map(|error| into_js_errors(input, error))
                .flatten()
                .collect::<Vec<_>>();

            JsValue::from_serde(&errors).unwrap()
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

#[derive(Clone)]
struct EventRelay {
    listeners: Rc<Mutex<HashMap<String, Vec<js_sys::Function>>>>,
    universal: Rc<Mutex<Vec<js_sys::Function>>>,
}

impl EventRelay {
    fn new() -> EventRelay {
        EventRelay {
            listeners: Rc::new(Mutex::new(HashMap::new())),
            universal: Rc::new(Mutex::new(Vec::new())),
        }
    }

    fn add_listener(&mut self, event: String, listener: js_sys::Function) {
        if event == "*" {
            self.universal
                .lock()
                .unwrap()
                .push(listener);
        } else {
            self.listeners
                .lock()
                .unwrap()
                .entry(event)
                .or_default()
                .push(listener);
        }
    }
}

impl EventListener for EventRelay {
    fn event(&mut self, event: &Event) {
        let name = match event {
            Event::SupervisorCall { .. } => "supervisor-call",
            Event::MemoryChange { .. } => "memory-change",
            Event::RegisterChange { .. } => "register-change",
            Event::Output { .. } => "output",
        };

        let universal = self.universal.lock().unwrap();
        let listeners = self.listeners.lock().unwrap();

        let listeners = listeners
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .chain(universal.iter());

        let object = match event {
            Event::SupervisorCall { code } => json!({
                "code": code
            }),
            Event::MemoryChange { address, data } => json!({
                "address": address,
                "data": data,
            }),
            Event::RegisterChange { register, data } => json!({
                "register": register.index(),
                "data": data,
            }),
            Event::Output { device, data } => json!({
                "device": device,
                "data": data,
            }),
        };

        let object = json!({
            "type": name,
            "payload": object,
        });

        let object = JsValue::from_serde(&object).unwrap();

        for listener in listeners {
            listener.call1(&JsValue::NULL, &object).unwrap();
        }
    }
}

#[wasm_bindgen]
pub struct WasmEmulator {
    emulator: Emulator<BalloonMemory, QueueIO>,
    source_map: SourceMap<LineSpan>,
    symbol_table: HashMap<String, u16>,
    relay: EventRelay,
}

#[wasm_bindgen]
impl WasmEmulator {
    pub fn registers(&self) -> Vec<i32> {
        self.emulator.context.r.to_vec()
    }

    pub fn get_program_counter(&self) -> u16 {
        self.emulator.context.pc
    }

    pub fn add_listener(&mut self, event: String, listener: js_sys::Function) {
        self.relay.add_listener(event, listener);
    }

    pub fn step(&mut self) -> Output {
        self.emulator.step().unwrap();

        let output = self.emulator.io.output.clone();
        let calls = self.emulator.io.calls.clone();

        let line = self.source_map.get_source_span(self.emulator.context.pc as usize)
            .map(|span| span.start.line)
            .unwrap_or(0);

        Output {
            output,
            calls,
            line: line as u32,
        }
    }

    pub fn stack_pointer(&self) -> u16 {
        self.emulator.context.r[6] as u16
    }

    pub fn read_address(&mut self, addr: u16) -> Result<i32, JsValue> {
        self.emulator.memory.get_data(addr)
            .map_err(|_| JsValue::from_serde(&json!({
                "error": "memory_error",
            })).unwrap())
    }

    /// Return an object that contains symbol names as keys and their memory
    /// addresses as the values.
    pub fn symbol_table(&self) -> JsValue {
        JsValue::from_serde(&self.symbol_table).unwrap()
    }

    /// Get source map of the currently loaded program as a map object that
    /// associates memory addresses (keys) with source code lines (values).
    pub fn source_map(&self) -> WasmSourceMap {
        WasmSourceMap(self.source_map.clone())
    }
}

#[wasm_bindgen]
pub struct WasmSourceMap(SourceMap<LineSpan>);

#[wasm_bindgen]
impl WasmSourceMap {
    fn get_source_line(&self, addr: usize) -> Result<usize, JsValue> {
        self.0.get_source_span(addr)
            .map(|span| span.start.line)
            .ok_or(JsValue::from_serde(&json!({
                "error": "source_map_error",
            })).unwrap())
    }
}

#[wasm_bindgen]
pub fn create_emulator(input: &str) -> WasmEmulator {
    let program = Program::parse(input).unwrap();
    let program = program.compile();
    // let result = program.compile_sourcemap();

    /*let source_map = result.source_map.into_iter()
        .map(|(address, span)| (address, calculate_position(input, span.start).0))
        .collect();*/

    let symbol_table = program.symbol_table.iter()
        .filter_map(|symbol| {
            let label = symbol.get::<Label>().into_owned();
            let value = symbol.get::<Value>().into_owned();

            match value {
                Some(value) => Some((label, value as u16)),
                None => None,
            }
        })
        .collect();

    let source_map = program.source_map.clone().into_line_based(input);

    let memory = BalloonMemory::new(program);
    let relay = EventRelay::new();

    let mut emulator = Emulator::new(memory, QueueIO::new())
        .unwrap();

    emulator.add_listener(relay.clone());

    WasmEmulator {
        emulator,
        source_map,
        relay,
        symbol_table,
    }
}

#[wasm_bindgen]
pub fn execute(asm: &str) -> Vec<i32> {
    let program = Program::parse(asm).unwrap();
    let compiled = program.compile();
    let memory = compiled.to_words();

    let mut io = TestIo::new();

    let mut emulator = Emulator::new(memory, &mut io)
        .unwrap();

    emulator.run().unwrap();

    io.into_output()
}

