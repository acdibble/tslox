use crate::chunk::*;
use crate::compiler::*;
use crate::native;
use crate::value::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryInto;

fn with_vm<T, F: FnOnce(&mut VM) -> T>(f: F) -> T {
    thread_local!(static STATIC_VM: RefCell<VM> = {
        RefCell::new(VM::new())
    });
    STATIC_VM.with(|vm| f(&mut *vm.borrow_mut()))
}

struct CallFrame {
    function: Function,
    ip: usize,
    starts_at: usize,
}

impl CallFrame {
    fn new(function: Function, starts_at: usize) -> CallFrame {
        CallFrame {
            function,
            starts_at,
            ip: 0,
        }
    }
}

#[derive(PartialEq)]
pub enum InterpretError {
    CompileError,
    RuntimeError,
    InternalError(&'static str),
}

#[derive(Default)]
pub struct VM {
    stack: Vec<Value>,
    globals: HashMap<&'static str, Value>,

    frames: Vec<CallFrame>,
}

type Result<T> = std::result::Result<T, InterpretError>;

pub fn interpret(source: &String) -> Result<()> {
    with_vm(|vm| {
        let function = compile(source)?;
        vm.stack.push(Value::Function(function.clone()));
        vm.call(function, 0).ok();
        vm.run()
    })
}

impl VM {
    pub fn new() -> VM {
        let mut vm: VM = Default::default();

        vm.define_native("clock", native::clock);

        vm
    }

    fn reset_stack(&mut self) {
        self.stack.clear();
        self.frames.clear()
    }

    #[inline(always)]
    fn current_frame(&self) -> &CallFrame {
        self.frames.last().unwrap()
    }

    #[inline(always)]
    fn current_frame_mut(&mut self) -> &mut CallFrame {
        self.frames.last_mut().unwrap()
    }

    #[inline(always)]
    fn current_chunk(&self) -> &Chunk {
        &self.current_frame().function.chunk
    }

    fn runtime_error<'a>(&mut self, string: &'a str) -> Result<()> {
        eprintln!("{}", string);

        for frame in self.frames.iter().rev() {
            let function = &frame.function;
            let line = function.chunk.lines[frame.ip - 1];

            eprintln!("[line {}] in {}()", line, function.get_name());
        }
        self.reset_stack();
        Err(InterpretError::RuntimeError)
    }

    fn define_native(&mut self, name: &'static str, function: native::Function) {
        self.globals.insert(name, Value::Native(function));
    }

    #[inline(always)]
    fn push(&mut self, value: Value) {
        self.stack.push(value)
    }

    #[inline(always)]
    fn pop(&mut self) -> Result<Value> {
        match self.stack.pop() {
            Some(value) => Ok(value),
            _ => Err(InterpretError::InternalError("Can't pop on empty stack.")),
        }
    }

    #[inline(always)]
    fn peek(&self, index: usize) -> Result<&Value> {
        let len = self.stack.len();
        match self.stack.get(len - 1 - index) {
            Some(value) => Ok(value),
            None => Err(InterpretError::InternalError("Can't peek on empty stack.")),
        }
    }

    #[inline(always)]
    fn call(&mut self, function: Function, arg_count: usize) -> Result<()> {
        if arg_count != function.arity {
            return self.runtime_error(
                format!(
                    "Expected {} arguments but got {}.",
                    function.arity, arg_count
                )
                .as_str(),
            );
        }

        let starts_at = self.stack.len() - arg_count - 1;
        self.frames.push(CallFrame::new(function, starts_at));
        Ok(())
    }

    #[inline(always)]
    fn call_native(&mut self, function: native::Function, arg_count: usize) -> Result<()> {
        let arg_start = self.stack.len() - arg_count - 1;
        let result = function(&self.stack[arg_start..]);
        self.stack.truncate(arg_start);
        self.stack.push(result);
        Ok(())
    }

    #[inline(always)]
    fn call_value(&mut self, callee: Value, arg_count: usize) -> Result<()> {
        match callee {
            Value::Function(function) => self.call(function, arg_count),
            Value::Native(function) => self.call_native(function, arg_count),
            _ => self.runtime_error("Can only call functions and classes."),
        }
    }

    fn run(&mut self) -> Result<()> {
        macro_rules! read_u8 {
            () => {{
                let frame = self.current_frame_mut();
                let ip = frame.ip;
                frame.ip += 1;
                let chunk = self.current_chunk();
                match chunk.code.get(ip) {
                    Some(byte) => Ok(*byte),
                    _ => return Err(InterpretError::InternalError("Failed to read byte.")),
                }
            }};
        }

        macro_rules! read_constant {
            () => {{
                let constant: usize = read_u8!()?.into();
                match self.current_chunk().constants.get(constant) {
                    Some(value) => Ok(value),
                    _ => return Err(InterpretError::InternalError("Failed to read constant.")),
                }
            }};
        }

        macro_rules! read_u16 {
            () => {{
                let byte1: u16 = read_u8!()?.into();
                let byte2: u16 = read_u8!()?.into();
                Ok((byte1 << 8) | byte2)
            }};
        }

        macro_rules! read_string {
            () => {
                match read_constant!()? {
                    Value::String(handle) => Ok(handle),
                    _ => return Err(InterpretError::InternalError("Value was not a string.")),
                }
            };
        }

        macro_rules! binary_op {
            ($op: tt, $variant: ident) => {{
                let value = match (self.peek(0)?, self.peek(1)?) {
                (Value::Number(b), Value::Number(a)) => (a $op b),
                _ => {
                    return self.runtime_error("Operands must be numbers.");
                }
                };

                self.pop()?;
                self.pop()?;
                self.push(Value::$variant(value))
            }};
        }

        loop {
            {
                #![cfg(feature = "trace-execution")]
                print!("          ");
                for value in self.stack.iter() {
                    print!("[ ");
                    value.print();
                    print!(" ]");
                }
                println!("");
                let ip = self.current_frame().ip;
                self.current_chunk().disassemble_instruction(ip);
            }

            let instruction = match read_u8!()?.try_into() {
                Ok(op) => op,
                Err(value) => {
                    let message = format!("Got unexpected instruction: '{}'", value);
                    return self.runtime_error(message.as_str());
                }
            };

            match instruction {
                Op::Constant => {
                    let constant = read_constant!()?;
                    {
                        #![cfg(feature = "trace-execution")]
                        constant.println();
                    }
                    let clone = constant.clone();
                    self.push(clone);
                }
                Op::Nil => self.push(Value::Nil),
                Op::True => self.push(Value::Bool(true)),
                Op::False => self.push(Value::Bool(false)),
                Op::Pop => {
                    self.pop()?;
                }
                Op::GetLocal => {
                    let slot: usize = read_u8!()?.into();
                    let offset = self.current_frame().starts_at;
                    self.push(self.stack[slot + offset].clone());
                }
                Op::SetLocal => {
                    let slot: usize = read_u8!()?.into();
                    let offset = self.current_frame().starts_at;
                    self.stack[slot + offset] = self.peek(0)?.clone();
                }
                Op::GetGlobal => {
                    let name = read_string!()?.as_str().string;
                    match self.globals.get(name) {
                        Some(value) => {
                            let clone = value.clone();
                            self.push(clone);
                        }
                        _ => {
                            let error = format!("Undefined variable '{}'.", name);
                            return self.runtime_error(error.as_str());
                        }
                    }
                }
                Op::DefineGlobal => {
                    let name = read_string!()?.as_str().string;
                    let value = self.pop()?;
                    self.globals.insert(name, value.clone());
                }
                Op::SetGlobal => {
                    let name = read_string!()?;
                    let string = name.as_str().string;
                    if self.globals.insert(string, self.peek(0)?.clone()).is_none() {
                        self.globals.remove(string);
                        let error = format!("Undefined variable '{}'.", string);
                        return self.runtime_error(error.as_str());
                    }
                }
                Op::Equal => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(a == b));
                }
                Op::Greater => binary_op!(>, Bool),
                Op::Less => binary_op!(<, Bool),
                Op::Add => {
                    let value = match (self.peek(0)?, self.peek(1)?) {
                        (Value::Number(b), Value::Number(a)) => Value::Number(a + b),
                        (Value::String(b), Value::String(a)) => Value::String(a + b),
                        _ => {
                            return self.runtime_error("Operands must be numbers.");
                        }
                    };

                    self.pop()?;
                    self.pop()?;
                    self.push(value);
                }
                Op::Subtract => binary_op!(-, Number),
                Op::Multiply => binary_op!(*, Number),
                Op::Divide => binary_op!(/, Number),
                Op::Not => {
                    let value = self.pop()?.is_falsy();
                    self.push(Value::Bool(value))
                }
                Op::Negate => {
                    let num = match self.peek(0)? {
                        Value::Number(num) => *num,
                        _ => {
                            return self.runtime_error("Operand must be a number.");
                        }
                    };
                    self.pop()?;
                    self.push(Value::Number(-num))
                }
                Op::Print => {
                    self.pop()?.println();
                }
                Op::Jump => {
                    let offset: usize = read_u16!()?.into();
                    let mut frame = self.current_frame_mut();
                    frame.ip += offset;
                }
                Op::JumpIfFalse => {
                    let offset: usize = read_u16!()?.into();
                    if self.peek(0)?.is_falsy() {
                        let frame = self.current_frame_mut();
                        frame.ip += offset
                    }
                }
                Op::Loop => {
                    let offset = read_u16!()?;
                    let frame = self.current_frame_mut();
                    frame.ip -= offset as usize;
                }
                Op::Call => {
                    let arg_count = read_u8!()? as usize;
                    let callee = self.peek(arg_count)?.clone();
                    self.call_value(callee, arg_count)?;
                }
                Op::Return => {
                    let result = self.pop()?;
                    let frame = self.frames.pop().unwrap();
                    if self.frames.len() == 0 {
                        self.pop()?;
                        return Ok(());
                    }

                    self.stack.truncate(frame.starts_at);
                    self.push(result)
                }
            }
        }
    }
}
