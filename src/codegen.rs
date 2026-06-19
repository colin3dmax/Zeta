//! Experimental native backend: MIR → LLVM IR → native code (cargo feature
//! `llvm`). Behind a feature so the default build/test needs no LLVM toolchain.
//! Targets the system LLVM 22 via inkwell `llvm22-1`.
//!
//! Build/run (arm64 macOS, system LLVM 22 from `brew install llvm`):
//!
//! ```sh
//! LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//!   cargo test --release --features llvm --lib codegen
//! ```
//!
//! The `llvm22-1-prefer-dynamic` inkwell feature links the single
//! libLLVM-22.dylib (which bundles zstd/z3/xml2), avoiding static component
//! libs whose Intel x86_64 copies under /usr/local shadow the arm64 ones.
//!
//! Supported subset: Int/Bool (i64), arithmetic/bitwise/comparison/logical, unary
//! ops, `let`/assignment, `if`/`while`/`break`/`continue`, user calls + recursion,
//! and **structs** (value semantics): struct literals, field read/write, struct
//! locals/params/returns, nesting. Strings (immutable `{len, ptr<i8>}`): literals,
//! `string_len`/`string_byte_at`/`string_byte_slice`/`string_concat`,
//! `int_to_string` (via libc snprintf), and the `ascii_is_*` predicates.
//! Enums (`{i64 tag, i64 p0, ptr p1}`, Int/Bool/String/array/struct/no-payload
//! variants; struct payloads are heap-boxed via p1) +
//! `match` (lowered to an LLVM `switch` over the tag, or over an Int/Bool value),
//! including string equality (`==`/`!=` via memcmp). `for` loops:
//! `for i in a..b`, `for x in intArray`, and C-style `for (init; cond; step)`.
//! Growable arrays via `{int,bool,string}_array_empty` / `_push` (functional
//! append: each push returns a fresh buffer, O(n) per push). Array ops are generic
//! over the element type (stride from `size_of`), so Int/Bool (i64) and String
//! (`{len,ptr}`) elements all work.

use crate::ast::{BinaryOp, StructDecl, UnaryOp};
use crate::mir::{MirExpr, MirFunction, MirPattern, MirPlace, MirStmt, Program};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::types::{BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{IntPredicate, OptimizationLevel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// JIT-compile a function `() -> i64` that returns `value`, run it, return the
/// result. Proves the inkwell ↔ LLVM 22 toolchain works end-to-end.
pub fn jit_smoke_constant(value: i64) -> i64 {
    let context = Context::create();
    let module = context.create_module("smoke");
    let builder = context.create_builder();

    let i64_type = context.i64_type();
    let fn_type = i64_type.fn_type(&[], false);
    let function = module.add_function("k", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    builder.position_at_end(entry);
    builder
        .build_return(Some(&i64_type.const_int(value as u64, true)))
        .unwrap();

    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT engine should initialize against LLVM 22");
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn() -> i64>("k")
            .expect("smoke function should be JIT-compiled");
        compiled.call()
    }
}

/// A Zeta value type as seen by codegen. Int and Bool are both `i64`; struct
/// types carry their declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ZType {
    Int,
    /// An f64 floating-point scalar (distinct from the i64 `Int`).
    Float,
    Struct(String),
    /// A dynamic array, represented at runtime as `{ i64 len, ptr data }` with
    /// `data` pointing at a heap (malloc) buffer of elements. Value semantics is
    /// realized by eagerly deep-copying the buffer at binding points (let /
    /// assignment / argument), so each owner has its own buffer — observably
    /// identical to the interpreter's copy-on-write. Only Int elements for now.
    Array(Box<ZType>),
    /// A string, represented at runtime as `{ i64 len, ptr<i8> data }` — the same
    /// `{len, ptr}` layout as [`ZType::Array`], but `data` points at a byte buffer
    /// and `len` is the byte count. Zeta strings are IMMUTABLE (no `s[i] = ...`),
    /// so multiple owners can share one read-only buffer — no deep copy at binding
    /// points is needed. Literals lower to a private global constant; `concat` /
    /// `byte_slice` allocate fresh malloc'd buffers.
    Str,
    /// An enum (tagged union), represented at runtime as `{ i64 tag, i64 p0, ptr p1 }`
    /// where `tag` is the variant's declaration index and `(p0, p1)` is a generic
    /// payload slot: Int/Bool use `p0`; String/array use `p0=len, p1=data` (the
    /// `{len, ptr}` split); a struct payload (too wide for the inline slot) is boxed
    /// on the heap with the pointer in `p1`. Payload-less variants leave it zero. The
    /// interpreter's by-name enum value is never observed directly, so this layout
    /// need not match.
    Enum(String),
    /// A tuple, represented as an LLVM anonymous struct `{T0, T1, ...}` whose
    /// element types are stored here in order. Like structs, tuple values live
    /// inline (insert_value / extract_value); fields are positional (`.0`, `.1`).
    Tuple(Vec<ZType>),
    /// A closure (function value), represented at runtime as `{ ptr fn, ptr env }`:
    /// `fn` points at a lifted top-level function whose first parameter is the
    /// heap-allocated environment of captured variables, and `env` is that
    /// environment. The carried `(param types, return type)` give the callee
    /// signature for indirect calls.
    Closure(Vec<ZType>, Box<ZType>),
}

/// Per-struct layout: field name → index (declaration order) and each field's
/// type, plus the LLVM struct type. Field ORDER is internal and need not match
/// the interpreter's by-name map — `main` returns an Int, so the differential
/// oracle never observes the layout.
#[derive(Clone)]
struct StructInfo<'ctx> {
    fields: Vec<(String, ZType)>,
    ty: StructType<'ctx>,
}

/// A generic struct declaration kept as a *template* (field types are raw type
/// STRINGS, still mentioning the type parameters). Each `Box<Int>` use site
/// monomorphizes it into a concrete `StructInfo` registered under the mangled
/// name `Box$Int` (see [`Types::instantiate_struct`]).
struct StructTemplate {
    type_params: Vec<String>,
    /// Field name → declared type string (e.g. `("value", "T")`).
    fields: Vec<(String, String)>,
}

/// A generic enum declaration kept as a *template* (payload types are raw type
/// STRINGS). Monomorphized per use into the `enums` instance table under a
/// mangled name like `Option$Int` (see [`Types::instantiate_enum`]).
struct EnumTemplate {
    type_params: Vec<String>,
    /// Variant name → optional payload type string, in declaration order.
    variants: Vec<(String, Option<String>)>,
}

struct Types<'ctx> {
    context: &'ctx Context,
    /// Concrete struct instances. Non-generic structs are keyed by their plain
    /// name; generic instances by a mangled name (`Box$Int`). Filled lazily at
    /// monomorphization time, hence the `RefCell`.
    structs: RefCell<HashMap<String, StructInfo<'ctx>>>,
    /// Enum name → variants in declaration order (the index is the runtime tag),
    /// each with its optional payload type. Keyed like `structs`: plain name for
    /// non-generic enums, mangled name (`Option$Int`) for generic instances.
    enums: RefCell<HashMap<String, Vec<(String, Option<ZType>)>>>,
    /// Generic struct/enum declarations, by base name, used to monomorphize on
    /// demand. Non-generic decls are NOT here (they are pre-built in `structs`/
    /// `enums`); only `type_params`-bearing decls need a template.
    struct_templates: HashMap<String, StructTemplate>,
    enum_templates: HashMap<String, EnumTemplate>,
    /// Function name → return type (so calls know their result type).
    returns: HashMap<String, ZType>,
}

impl<'ctx> Types<'ctx> {
    fn build(
        context: &'ctx Context,
        struct_decls: &[StructDecl],
        program: &Program,
    ) -> Result<Self, String> {
        // Only non-generic decls are pre-built into the instance tables; their
        // names are what `parse_ztype` resolves to `Struct`/`Enum`. Generic decls
        // are set aside as templates and monomorphized on demand at use sites.
        let names: Vec<&str> = struct_decls
            .iter()
            .filter(|d| d.type_params.is_empty())
            .map(|d| d.name.as_str())
            .collect();
        let enum_names: Vec<&str> = program
            .enums
            .iter()
            .filter(|e| e.type_params.is_empty())
            .map(|e| e.name.as_str())
            .collect();
        // Pass 1: opaque named struct types (so fields can reference each other).
        let mut opaque: HashMap<String, StructType> = HashMap::new();
        for decl in struct_decls.iter().filter(|d| d.type_params.is_empty()) {
            opaque.insert(decl.name.clone(), context.opaque_struct_type(&decl.name));
        }
        // Pass 2: resolve field types and set bodies (non-generic structs only).
        let mut structs = HashMap::new();
        let mut struct_templates = HashMap::new();
        for decl in struct_decls {
            if !decl.type_params.is_empty() {
                struct_templates.insert(
                    decl.name.clone(),
                    StructTemplate {
                        type_params: decl.type_params.clone(),
                        fields: decl
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.ty.clone()))
                            .collect(),
                    },
                );
                continue;
            }
            let mut fields = Vec::with_capacity(decl.fields.len());
            let mut field_llvm: Vec<BasicTypeEnum> = Vec::with_capacity(decl.fields.len());
            for field in &decl.fields {
                let zt = parse_ztype(&field.ty, &names, &enum_names)?;
                field_llvm.push(llvm_type_of(context, &zt, &opaque));
                fields.push((field.name.clone(), zt));
            }
            let ty = opaque[&decl.name];
            ty.set_body(&field_llvm, false);
            structs.insert(decl.name.clone(), StructInfo { fields, ty });
        }

        // Enum tables: variant order (= tag) + each variant's optional payload type.
        let mut enums = HashMap::new();
        let mut enum_templates = HashMap::new();
        for enum_decl in &program.enums {
            if !enum_decl.type_params.is_empty() {
                enum_templates.insert(
                    enum_decl.name.clone(),
                    EnumTemplate {
                        type_params: enum_decl.type_params.clone(),
                        variants: enum_decl
                            .variants
                            .iter()
                            .map(|v| (v.name.clone(), v.payload_type.clone()))
                            .collect(),
                    },
                );
                continue;
            }
            let mut variants = Vec::with_capacity(enum_decl.variants.len());
            for variant in &enum_decl.variants {
                let payload = match &variant.payload_type {
                    Some(t) => Some(parse_ztype(t, &names, &enum_names)?),
                    None => None,
                };
                variants.push((variant.name.clone(), payload));
            }
            enums.insert(enum_decl.name.clone(), variants);
        }

        // Construct first with empty returns, then resolve return types via
        // `resolve_ann_ztype` — which may monomorphize generic aggregate return
        // types (`Option<Int>`), registering instances into the (now live) tables.
        let mut types = Types {
            context,
            structs: RefCell::new(structs),
            enums: RefCell::new(enums),
            struct_templates,
            enum_templates,
            returns: HashMap::new(),
        };
        let mut returns = HashMap::new();
        for function in &program.functions {
            let zt = match &function.return_type {
                Some(t) => types.resolve_ann_ztype(t).unwrap_or(ZType::Int),
                None => ZType::Int, // Unit-returning → i64 0
            };
            returns.insert(function.name.clone(), zt);
        }
        types.returns = returns;
        Ok(types)
    }

    /// Resolve a declared type string to a `ZType`, using this module's known
    /// struct and enum names.
    fn parse_ztype(&self, text: &str) -> Result<ZType, String> {
        let structs = self.structs.borrow();
        let enums = self.enums.borrow();
        let struct_names: Vec<&str> = structs.keys().map(|s| s.as_str()).collect();
        let enum_names: Vec<&str> = enums.keys().map(|s| s.as_str()).collect();
        parse_ztype(text, &struct_names, &enum_names)
    }

    /// The LLVM type of an already-registered struct instance (`name` is a key in
    /// the `structs` table — plain for non-generic, mangled for generic).
    fn struct_llvm(&self, name: &str) -> StructType<'ctx> {
        self.structs.borrow()[name].ty
    }

    fn llvm(&self, zt: &ZType) -> BasicTypeEnum<'ctx> {
        match zt {
            ZType::Int => self.context.i64_type().into(),
            ZType::Float => self.context.f64_type().into(),
            ZType::Struct(name) => self.struct_llvm(name).into(),
            ZType::Array(_) | ZType::Str => array_struct_type(self.context).into(),
            ZType::Enum(_) => enum_struct_type(self.context).into(),
            ZType::Tuple(elements) => {
                let field_types: Vec<BasicTypeEnum<'ctx>> =
                    elements.iter().map(|e| self.llvm(e)).collect();
                self.context.struct_type(&field_types, false).into()
            }
            ZType::Closure(_, _) => closure_struct_type(self.context).into(),
        }
    }

    /// Resolve `enum_name.variant` to `(tag, payload_type)`; tag is the variant's
    /// declaration index. `enum_name` is a registered instance key (plain for a
    /// non-generic enum, mangled like `Option$Int` for a generic instance).
    fn variant_tag(&self, enum_name: &str, variant: &str) -> Result<(u64, Option<ZType>), String> {
        let enums = self.enums.borrow();
        let variants = enums
            .get(enum_name)
            .ok_or_else(|| format!("unknown enum `{enum_name}`"))?;
        variants
            .iter()
            .position(|(name, _)| name == variant)
            .map(|i| (i as u64, variants[i].1.clone()))
            .ok_or_else(|| format!("enum `{enum_name}` has no variant `{variant}`"))
    }

    /// The declaration index (= runtime tag) of `variant`, looked up by base name.
    /// Works for both registered instances and generic templates, since the
    /// variant order is identical across all instantiations.
    fn variant_index(&self, base: &str, variant: &str) -> Result<u64, String> {
        if let Some(tmpl) = self.enum_templates.get(base) {
            return tmpl
                .variants
                .iter()
                .position(|(name, _)| name == variant)
                .map(|i| i as u64)
                .ok_or_else(|| format!("enum `{base}` has no variant `{variant}`"));
        }
        self.variant_tag(base, variant).map(|(tag, _)| tag)
    }

    fn field_index(&self, struct_name: &str, field: &str) -> Result<(u32, ZType), String> {
        let structs = self.structs.borrow();
        let info = structs
            .get(struct_name)
            .ok_or_else(|| format!("unknown struct `{struct_name}`"))?;
        info.fields
            .iter()
            .position(|(name, _)| name == field)
            .map(|i| (i as u32, info.fields[i].1.clone()))
            .ok_or_else(|| format!("unknown field `{field}` on `{struct_name}`"))
    }

    /// Whether `name` is a generic struct base (has a template).
    fn is_generic_struct(&self, name: &str) -> bool {
        self.struct_templates.contains_key(name)
    }

    /// Whether `name` is a generic enum base (has a template).
    fn is_generic_enum(&self, name: &str) -> bool {
        self.enum_templates.contains_key(name)
    }

    /// Owned copy of a generic struct template's `(type_params, fields)` — taken
    /// before lowering field values so no borrow into `self.types` is held across
    /// the `&mut self` lowering calls.
    fn struct_template_of(&self, base: &str) -> (Vec<String>, Vec<(String, String)>) {
        let t = &self.struct_templates[base];
        (t.type_params.clone(), t.fields.clone())
    }

    /// Owned copy of a generic enum template's `type_params`.
    fn enum_template_params(&self, base: &str) -> Vec<String> {
        self.enum_templates[base].type_params.clone()
    }

    /// The payload type string declared for `base`'s `variant` (raw, may name a
    /// type parameter), if any.
    fn enum_variant_payload_str(&self, base: &str, variant: &str) -> Option<String> {
        self.enum_templates
            .get(base)?
            .variants
            .iter()
            .find(|(name, _)| name == variant)
            .and_then(|(_, p)| p.clone())
    }

    /// Field names of a registered struct instance, in declaration order.
    fn struct_field_names(&self, name: &str) -> Vec<String> {
        self.structs.borrow()[name]
            .fields
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// Resolve a TYPE ANNOTATION string to a `ZType`, monomorphizing generic
    /// aggregate instantiations (`Box<Int>`, `Option<Int>`, `Result<Int,String>`)
    /// on the fly and registering their instances. Falls back to `parse_ztype`
    /// for plain (non-generic) type strings.
    fn resolve_ann_ztype(&self, ann: &str) -> Result<ZType, String> {
        if let Some((params, ret)) = crate::type_syntax::fn_parts(ann) {
            let ptys = params
                .iter()
                .map(|p| self.resolve_ann_ztype(p))
                .collect::<Result<Vec<_>, _>>()?;
            let rty = self.resolve_ann_ztype(ret)?;
            return Ok(ZType::Closure(ptys, Box::new(rty)));
        }
        if let Some(parts) = crate::type_syntax::tuple_parts(ann) {
            let elems = parts
                .iter()
                .map(|p| self.resolve_ann_ztype(p))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ZType::Tuple(elems));
        }
        if let Some((base, arg_strs)) = crate::type_syntax::generic_parts(ann) {
            let args = arg_strs
                .iter()
                .map(|a| self.resolve_ann_ztype(a))
                .collect::<Result<Vec<_>, _>>()?;
            if self.is_generic_struct(base) {
                return Ok(ZType::Struct(self.instantiate_struct(base, &args)?));
            }
            if self.is_generic_enum(base) {
                return Ok(ZType::Enum(self.instantiate_enum(base, &args)?));
            }
            // Unknown generic base — best-effort parse of the base name.
            return self.parse_ztype(base);
        }
        self.parse_ztype(ann)
    }

    /// Resolve a template field/payload type string under a `T → ZType` mapping.
    /// A bare type-parameter name resolves to its bound concrete type; a nested
    /// generic instantiation (`Box<T>`) has its arguments substituted and is
    /// itself monomorphized; anything else is an ordinary concrete type string.
    fn resolve_template_type(
        &self,
        ty_str: &str,
        subst: &HashMap<String, ZType>,
    ) -> Result<ZType, String> {
        if let Some(zt) = subst.get(ty_str) {
            return Ok(zt.clone());
        }
        if let Some((base, arg_strs)) = crate::type_syntax::generic_parts(ty_str) {
            if self.is_generic_struct(base) || self.is_generic_enum(base) {
                let args = arg_strs
                    .iter()
                    .map(|a| self.resolve_template_type(a, subst))
                    .collect::<Result<Vec<_>, _>>()?;
                if self.is_generic_struct(base) {
                    return Ok(ZType::Struct(self.instantiate_struct(base, &args)?));
                }
                return Ok(ZType::Enum(self.instantiate_enum(base, &args)?));
            }
        }
        self.resolve_ann_ztype(ty_str)
    }

    /// Build the `T → ZType` substitution for a generic aggregate from its
    /// declared `type_params` and the concrete instantiation `args`. A param the
    /// args don't cover (e.g. the `E` of `Result.Ok(x)` at a construction site)
    /// defaults to `Int` — a harmless inline-slot-compatible placeholder, since
    /// that variant's payload is never extracted for this value.
    fn subst_of(type_params: &[String], args: &[ZType]) -> HashMap<String, ZType> {
        type_params
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), args.get(i).cloned().unwrap_or(ZType::Int)))
            .collect()
    }

    /// Monomorphize a generic struct at `args`, registering a concrete
    /// `StructInfo` under the mangled name (`Box$Int`) if not already present.
    /// Returns the registered instance name. A non-generic name passes through.
    fn instantiate_struct(&self, base: &str, args: &[ZType]) -> Result<String, String> {
        let Some(tmpl) = self.struct_templates.get(base) else {
            return Ok(base.to_string());
        };
        let mangled = mangle_instance(base, args);
        if self.structs.borrow().contains_key(&mangled) {
            return Ok(mangled);
        }
        let subst = Self::subst_of(&tmpl.type_params, args);
        let mut fields = Vec::with_capacity(tmpl.fields.len());
        let mut field_llvm: Vec<BasicTypeEnum> = Vec::with_capacity(tmpl.fields.len());
        for (fname, fty) in &tmpl.fields {
            let zt = self.resolve_template_type(fty, &subst)?;
            field_llvm.push(self.llvm(&zt));
            fields.push((fname.clone(), zt));
        }
        // Anonymous struct (generic instances aren't user-nameable, and the
        // differential oracle never observes the layout).
        let ty = self.context.struct_type(&field_llvm, false);
        self.structs
            .borrow_mut()
            .insert(mangled.clone(), StructInfo { fields, ty });
        Ok(mangled)
    }

    /// Monomorphize a generic enum at `args`, registering its concrete variant
    /// payload table under the mangled name (`Option$Int`) if absent. Returns the
    /// registered instance name. A non-generic name passes through.
    fn instantiate_enum(&self, base: &str, args: &[ZType]) -> Result<String, String> {
        let Some(tmpl) = self.enum_templates.get(base) else {
            return Ok(base.to_string());
        };
        let mangled = mangle_instance(base, args);
        if self.enums.borrow().contains_key(&mangled) {
            return Ok(mangled);
        }
        let subst = Self::subst_of(&tmpl.type_params, args);
        let mut variants = Vec::with_capacity(tmpl.variants.len());
        for (vname, payload) in &tmpl.variants {
            let zt = match payload {
                Some(p) => Some(self.resolve_template_type(p, &subst)?),
                None => None,
            };
            variants.push((vname.clone(), zt));
        }
        self.enums.borrow_mut().insert(mangled.clone(), variants);
        Ok(mangled)
    }
}

fn parse_ztype(text: &str, struct_names: &[&str], enum_names: &[&str]) -> Result<ZType, String> {
    if let Some((params, ret)) = crate::type_syntax::fn_parts(text) {
        let param_types = params
            .iter()
            .map(|p| parse_ztype(p, struct_names, enum_names))
            .collect::<Result<Vec<_>, _>>()?;
        let ret_type = parse_ztype(ret, struct_names, enum_names)?;
        return Ok(ZType::Closure(param_types, Box::new(ret_type)));
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(text) {
        let elems = parts
            .iter()
            .map(|p| parse_ztype(p, struct_names, enum_names))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(ZType::Tuple(elems));
    }
    match text {
        "Int" | "Bool" => Ok(ZType::Int),
        "Float" => Ok(ZType::Float),
        "Unit" => Ok(ZType::Int),
        "String" => Ok(ZType::Str),
        "IntArray" | "BoolArray" => Ok(ZType::Array(Box::new(ZType::Int))),
        "StringArray" => Ok(ZType::Array(Box::new(ZType::Str))),
        "FloatArray" => Ok(ZType::Array(Box::new(ZType::Float))),
        name if struct_names.contains(&name) => Ok(ZType::Struct(name.to_string())),
        name if enum_names.contains(&name) => Ok(ZType::Enum(name.to_string())),
        other => Err(format!("type `{other}` not in the native subset")),
    }
}

/// Whether `expr` evaluates to a freshly-allocated, unaliased array buffer (so a
/// binding need not deep-copy it for value semantics): an array literal or an
/// `*_array_empty` / `*_array_push` builtin result.
fn is_fresh_array(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::ArrayLiteral { .. } => true,
        MirExpr::Call { callee, .. } => matches!(
            callee.as_str(),
            "int_array_empty"
                | "int_array_push"
                | "bool_array_empty"
                | "bool_array_push"
                | "string_array_empty"
                | "string_array_push"
                | "float_array_empty"
                | "float_array_push"
        ),
        _ => false,
    }
}

/// Recognize the in-place push idiom `name = <T>_array_push(name, value)` —
/// returns `(name, element type, value expr)` when `place` is exactly the local
/// the push reads as its first argument.
fn match_inplace_push<'p>(place: &'p MirPlace, value: &'p MirExpr) -> Option<(&'p str, ZType, &'p MirExpr)> {
    let MirPlace::Local(name) = place else {
        return None;
    };
    let MirExpr::Call { callee, args } = value else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    let elem = match callee.as_str() {
        "int_array_push" | "bool_array_push" => ZType::Int,
        "string_array_push" => ZType::Str,
        "float_array_push" => ZType::Float,
        _ => return None,
    };
    // First arg must be `Load(name)` — the same variable being assigned.
    match &args[0] {
        MirExpr::Load(arg_name) if arg_name == name => Some((name, elem, &args[1])),
        _ => None,
    }
}

/// The `{ i64 len, ptr data }` value type used for all arrays.
fn array_struct_type(context: &Context) -> StructType {
    context.struct_type(
        &[
            context.i64_type().into(),
            context.ptr_type(inkwell::AddressSpace::default()).into(),
        ],
        false,
    )
}

/// The `{ ptr fn, ptr env }` value type used for all closures.
fn closure_struct_type(context: &Context) -> StructType {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    context.struct_type(&[ptr.into(), ptr.into()], false)
}

/// Unify a generic parameter type STRING against a concrete `ZType`, binding any
/// type parameters it names into `subst` (recursing through tuple/function types).
fn unify_ztype(
    generic_str: &str,
    concrete: &ZType,
    type_params: &[String],
    subst: &mut HashMap<String, ZType>,
) {
    if type_params.iter().any(|p| p == generic_str) {
        subst.insert(generic_str.to_string(), concrete.clone());
        return;
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(generic_str) {
        if let ZType::Tuple(elems) = concrete {
            for (p, c) in parts.iter().zip(elems) {
                unify_ztype(p, c, type_params, subst);
            }
        }
        return;
    }
    if let Some((params, ret)) = crate::type_syntax::fn_parts(generic_str) {
        if let ZType::Closure(cparams, cret) = concrete {
            for (p, c) in params.iter().zip(cparams) {
                unify_ztype(p, c, type_params, subst);
            }
            unify_ztype(ret, cret, type_params, subst);
        }
    }
}

/// A deterministic, collision-resistant mangle of a `ZType` for instance naming.
fn zty_mangle(zt: &ZType) -> String {
    match zt {
        ZType::Int => "Int".to_string(),
        ZType::Float => "Float".to_string(),
        ZType::Str => "Str".to_string(),
        ZType::Struct(name) => format!("S{}", name),
        ZType::Enum(name) => format!("E{}", name),
        ZType::Array(elem) => format!("Arr{}", zty_mangle(elem)),
        ZType::Tuple(elems) => {
            let inner: Vec<String> = elems.iter().map(zty_mangle).collect();
            format!("Tup{}_{}e", inner.len(), inner.join("_"))
        }
        ZType::Closure(params, ret) => {
            let inner: Vec<String> = params.iter().map(zty_mangle).collect();
            format!("Fn{}_{}_r{}", inner.len(), inner.join("_"), zty_mangle(ret))
        }
    }
}

/// Mangled name for a monomorphized instance, e.g. `id$Int`, `pair$Int_Bool`.
fn mangle_instance(callee: &str, arg_ztys: &[ZType]) -> String {
    let parts: Vec<String> = arg_ztys.iter().map(zty_mangle).collect();
    format!("{callee}${}", parts.join("_"))
}

/// Collect the free `Load` names of `expr` — names it reads that are not bound by
/// an enclosing lambda's parameters. Order is first-seen (deterministic). Used to
/// decide which enclosing locals a lambda must capture into its environment.
fn collect_free_loads(expr: &MirExpr, bound: &mut HashSet<String>, out: &mut Vec<String>) {
    match expr {
        MirExpr::Load(name) => {
            if !bound.contains(name) && !out.contains(name) {
                out.push(name.clone());
            }
        }
        MirExpr::Int(_) | MirExpr::Float(_) | MirExpr::String(_) | MirExpr::Bool(_) => {}
        MirExpr::Binary { left, right, .. } => {
            collect_free_loads(left, bound, out);
            collect_free_loads(right, bound, out);
        }
        MirExpr::Unary { expr, .. } => collect_free_loads(expr, bound, out),
        MirExpr::Call { args, .. } => {
            for arg in args {
                collect_free_loads(arg, bound, out);
            }
        }
        MirExpr::EnumVariant { payload, .. } => {
            if let Some(payload) = payload {
                collect_free_loads(payload, bound, out);
            }
        }
        MirExpr::StructLiteral { fields, .. } => {
            for field in fields {
                collect_free_loads(&field.value, bound, out);
            }
        }
        MirExpr::FieldAccess { base, .. } => collect_free_loads(base, bound, out),
        MirExpr::ArrayLiteral { elements } | MirExpr::Tuple { elements } => {
            for element in elements {
                collect_free_loads(element, bound, out);
            }
        }
        MirExpr::Lambda { params, body } => {
            // A nested lambda's params are bound inside its body; everything else
            // it reads is also free in this lambda (transitive capture).
            let added: Vec<String> = params
                .iter()
                .map(|p| p.name.clone())
                .filter(|n| bound.insert(n.clone()))
                .collect();
            collect_free_loads(body, bound, out);
            for name in added {
                bound.remove(&name);
            }
        }
        MirExpr::Index { base, index } => {
            collect_free_loads(base, bound, out);
            collect_free_loads(index, bound, out);
        }
    }
}

/// The `{ i64 tag, i64 p0, ptr p1 }` value type used for all enums. `(p0, p1)` is
/// a generic payload slot (Int in p0; String/array's `{len, ptr}` split across
/// p0/p1; a struct payload is heap-boxed with the pointer in p1).
fn enum_struct_type(context: &Context) -> StructType {
    let i64_ty = context.i64_type();
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    context.struct_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
}

fn llvm_type_of<'ctx>(
    context: &'ctx Context,
    zt: &ZType,
    opaque: &HashMap<String, StructType<'ctx>>,
) -> BasicTypeEnum<'ctx> {
    match zt {
        ZType::Int => context.i64_type().into(),
        ZType::Float => context.f64_type().into(),
        ZType::Struct(name) => opaque[name].into(),
        ZType::Array(_) | ZType::Str => array_struct_type(context).into(),
        ZType::Enum(_) => enum_struct_type(context).into(),
        ZType::Tuple(elements) => {
            let field_types: Vec<BasicTypeEnum<'ctx>> = elements
                .iter()
                .map(|e| llvm_type_of(context, e, opaque))
                .collect();
            context.struct_type(&field_types, false).into()
        }
        ZType::Closure(_, _) => closure_struct_type(context).into(),
    }
}

/// JIT-compile `program` and run its no-arg, Int-returning `entry` to an `i64`.
/// The Stage0 interpreter (`run_mir`) is the differential oracle.
pub fn jit_run_i64(program: &Program, structs: &[StructDecl], entry: &str) -> Result<i64, String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn() -> i64>(entry)
            .map_err(|e| format!("entry `{entry}` not found: {e}"))?;
        Ok(compiled.call())
    }
}

/// Emit the textual LLVM IR for `program` (no JIT/run). Exposed for tests that
/// inspect the generated IR (e.g. that array locals are freed at scope exit).
pub fn emit_llvm_ir(program: &Program, structs: &[StructDecl]) -> Result<String, String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    Ok(module.print_to_string().to_string())
}

/// Like [`jit_run_i64`] but the entry takes one `i64` argument and the module is
/// run through LLVM `-O3` before JIT — real optimized native code. The runtime
/// `arg` keeps the optimizer from constant-folding the computation away.
pub fn jit_run_i64_arg(
    program: &Program,
    structs: &[StructDecl],
    entry: &str,
    arg: i64,
) -> Result<i64, String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    optimize_module(&module)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn(i64) -> i64>(entry)
            .map_err(|e| format!("entry `{entry}` not found: {e}"))?;
        Ok(compiled.call(arg))
    }
}

/// Compile `entry` (one `i64` arg) to optimized native, then time ONLY the call.
pub fn jit_time_i64_arg(
    program: &Program,
    structs: &[StructDecl],
    entry: &str,
    arg: i64,
) -> Result<(i64, std::time::Duration), String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    optimize_module(&module)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn(i64) -> i64>(entry)
            .map_err(|e| format!("entry `{entry}` not found: {e}"))?;
        let start = std::time::Instant::now();
        let result = compiled.call(arg);
        let elapsed = start.elapsed();
        Ok((result, elapsed))
    }
}

/// A long-running service whose `step` runs as **optimized native code** and can
/// be hot-swapped without losing state (native counterpart of
/// `runtime::ServiceDriver`; see docs/compiler/hot-reload-design.md). Convention:
/// `fn init() -> Int` and `reloadable fn step(state: Int, input: Int) -> Int`.
///
/// The accumulated state lives here (an `i64`); each tick calls the current
/// native `step`. `reload` JIT-compiles a new program to native and atomically
/// repoints `step` — the state is untouched, so the new (native-speed) code
/// resumes from it. This realizes the §3.1 picture: the hot path is native, only
/// the `step` boundary is an indirect call.
pub struct NativeService {
    // Leaked context keeps the JIT'd code's types alive for the engine's life.
    engine: inkwell::execution_engine::ExecutionEngine<'static>,
    step_addr: usize,
    state: i64,
}

impl NativeService {
    pub fn start(program: &Program, structs: &[StructDecl]) -> Result<NativeService, String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let init_addr = engine
            .get_function_address("init")
            .map_err(|e| format!("`init` not found: {e}"))?;
        let init: extern "C" fn() -> i64 = unsafe { std::mem::transmute(init_addr) };
        let state = init();
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        Ok(NativeService {
            engine,
            step_addr,
            state,
        })
    }

    /// Advance one tick by calling the current native `step(state, input)`.
    pub fn tick(&mut self, input: i64) -> i64 {
        let step: extern "C" fn(i64, i64) -> i64 = unsafe { std::mem::transmute(self.step_addr) };
        self.state = step(self.state, input);
        self.state
    }

    pub fn state(&self) -> i64 {
        self.state
    }

    /// Hot-swap to a freshly JIT-compiled native program. State is preserved.
    pub fn reload(&mut self, program: &Program, structs: &[StructDecl]) -> Result<(), String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        // Point at the new code BEFORE dropping the old engine (which unmaps the
        // old code); the preserved `state` i64 carries straight over.
        self.step_addr = step_addr;
        self.engine = engine;
        Ok(())
    }
}

/// The `{ i64 len, ptr data }` array value, laid out for the C ABI so it can
/// cross the Rust↔native boundary (returned/passed in two registers on arm64).
/// The `data` buffer is libc-malloc'd by the JIT'd code, so it lives on the C
/// heap and SURVIVES an engine swap — only the code is unmapped on reload.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeArray {
    pub len: i64,
    pub data: *mut i64,
}

/// Like [`NativeService`] but the threaded state is an `IntArray`, proving native
/// hot-reload works with a non-scalar, heap-backed state. Convention:
/// `fn init() -> IntArray` and `reloadable fn step(state: IntArray, input: Int) -> IntArray`.
pub struct NativeArrayService {
    engine: inkwell::execution_engine::ExecutionEngine<'static>,
    step_addr: usize,
    state: NativeArray,
}

impl NativeArrayService {
    pub fn start(program: &Program, structs: &[StructDecl]) -> Result<NativeArrayService, String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let init_addr = engine
            .get_function_address("init")
            .map_err(|e| format!("`init` not found: {e}"))?;
        let init: extern "C" fn() -> NativeArray = unsafe { std::mem::transmute(init_addr) };
        let state = init();
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        Ok(NativeArrayService {
            engine,
            step_addr,
            state,
        })
    }

    pub fn tick(&mut self, input: i64) {
        let step: extern "C" fn(NativeArray, i64) -> NativeArray =
            unsafe { std::mem::transmute(self.step_addr) };
        self.state = step(self.state, input);
    }

    pub fn len(&self) -> i64 {
        self.state.len
    }

    /// Read element `i` of the current state buffer.
    pub fn get(&self, i: i64) -> i64 {
        assert!(i >= 0 && i < self.state.len, "index out of bounds");
        unsafe { *self.state.data.offset(i as isize) }
    }

    pub fn reload(&mut self, program: &Program, structs: &[StructDecl]) -> Result<(), String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        self.step_addr = step_addr;
        self.engine = engine;
        Ok(())
    }
}

/// Like [`NativeService`] but the threaded state is a **struct** of arbitrary
/// shape. To sidestep the per-struct C ABI (registers vs. sret), the module gets
/// two pointer-based wrappers — `__svc_init(out)` and `__svc_step(state, input,
/// out)` — that load/store the struct through pointers. The state lives in a
/// Rust-owned, 8-byte-aligned buffer (so it survives an engine swap), and ticks
/// ping-pong between two buffers. Convention: `fn init() -> S` and
/// `reloadable fn step(state: S, input: Int) -> S` for some struct `S`.
pub struct NativeStructService {
    engine: inkwell::execution_engine::ExecutionEngine<'static>,
    step_addr: usize,
    state: Vec<i64>,
    scratch: Vec<i64>,
    /// Byte offset of each struct field (for reading Int fields out of the blob).
    field_offsets: Vec<u64>,
}

impl NativeStructService {
    pub fn start(program: &Program, structs: &[StructDecl]) -> Result<NativeStructService, String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let (engine, words, field_offsets) = compile_struct_service(context, program, structs)?;
        let mut state = vec![0i64; words];
        let init: extern "C" fn(*mut i64) = unsafe {
            std::mem::transmute(
                engine
                    .get_function_address("__svc_init")
                    .map_err(|e| format!("`__svc_init` not found: {e}"))?,
            )
        };
        init(state.as_mut_ptr());
        let step_addr = engine
            .get_function_address("__svc_step")
            .map_err(|e| format!("`__svc_step` not found: {e}"))?;
        Ok(NativeStructService {
            engine,
            step_addr,
            scratch: vec![0i64; words],
            state,
            field_offsets,
        })
    }

    /// Advance one tick: `state' = step(state, input)`, via the pointer wrapper.
    pub fn tick(&mut self, input: i64) {
        let step: extern "C" fn(*const i64, i64, *mut i64) =
            unsafe { std::mem::transmute(self.step_addr) };
        step(self.state.as_ptr(), input, self.scratch.as_mut_ptr());
        std::mem::swap(&mut self.state, &mut self.scratch);
    }

    /// Read struct field `index` (an Int field) out of the current state blob.
    pub fn field_i64(&self, index: usize) -> i64 {
        let offset = self.field_offsets[index] as usize;
        unsafe { *((self.state.as_ptr() as *const u8).add(offset) as *const i64) }
    }

    pub fn reload(&mut self, program: &Program, structs: &[StructDecl]) -> Result<(), String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let (engine, _words, _offsets) = compile_struct_service(context, program, structs)?;
        let step_addr = engine
            .get_function_address("__svc_step")
            .map_err(|e| format!("`__svc_step` not found: {e}"))?;
        // State blob is Rust-owned, so it carries over untouched.
        self.step_addr = step_addr;
        self.engine = engine;
        Ok(())
    }
}

/// Build the module, append the pointer-based service wrappers, optimize, and
/// wrap in a JIT engine; also return the state struct's word count and field byte
/// offsets (queried from the JIT target data).
fn compile_struct_service(
    context: &'static Context,
    program: &Program,
    structs: &[StructDecl],
) -> Result<
    (
        inkwell::execution_engine::ExecutionEngine<'static>,
        usize,
        Vec<u64>,
    ),
    String,
> {
    let types = Types::build(context, structs, program)?;
    let module = build_module(context, &types, program)?;
    let ZType::Struct(state_name) = &types.returns["init"] else {
        return Err("NativeStructService requires `init` to return a struct".into());
    };
    let struct_ty = types.struct_llvm(state_name);
    add_struct_service_wrappers(context, &module, struct_ty)?;
    optimize_module(&module)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    let td = engine.get_target_data();
    let size = td.get_store_size(&struct_ty);
    let words = ((size + 7) / 8) as usize;
    let field_offsets = (0..struct_ty.count_fields())
        .map(|i| td.offset_of_element(&struct_ty, i).unwrap_or(0))
        .collect();
    Ok((engine, words.max(1), field_offsets))
}

/// Add `__svc_init(out*)` and `__svc_step(state*, input, out*)` to `module`,
/// loading/storing the state struct through pointers so the Rust side can use a
/// single pointer ABI regardless of the struct's size.
fn add_struct_service_wrappers<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    struct_ty: StructType<'ctx>,
) -> Result<(), String> {
    let builder = context.create_builder();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let void = context.void_type();
    let init = module.get_function("init").ok_or("`init` not found")?;
    let step = module.get_function("step").ok_or("`step` not found")?;

    let init_w = module.add_function("__svc_init", void.fn_type(&[ptr.into()], false), None);
    let bb = context.append_basic_block(init_w, "entry");
    builder.position_at_end(bb);
    let out = init_w.get_nth_param(0).unwrap().into_pointer_value();
    let s = builder
        .build_call(init, &[], "s")
        .unwrap()
        .try_as_basic_value()
        .basic()
        .ok_or("`init` returned no value")?;
    builder.build_store(out, s).unwrap();
    builder.build_return(None).unwrap();

    let step_w = module.add_function(
        "__svc_step",
        void.fn_type(&[ptr.into(), i64_ty.into(), ptr.into()], false),
        None,
    );
    let bb = context.append_basic_block(step_w, "entry");
    builder.position_at_end(bb);
    let statep = step_w.get_nth_param(0).unwrap().into_pointer_value();
    let input = step_w.get_nth_param(1).unwrap();
    let out = step_w.get_nth_param(2).unwrap().into_pointer_value();
    let state = builder.build_load(struct_ty, statep, "st").unwrap();
    let r = builder
        .build_call(step, &[state.into(), input.into()], "r")
        .unwrap()
        .try_as_basic_value()
        .basic()
        .ok_or("`step` returned no value")?;
    builder.build_store(out, r).unwrap();
    builder.build_return(None).unwrap();

    module
        .verify()
        .map_err(|e| format!("service wrapper verification failed: {e}"))
}

/// Build + optimize a module and wrap it in an aggressive JIT engine.
fn compile_engine(
    context: &'static Context,
    program: &Program,
    structs: &[StructDecl],
) -> Result<inkwell::execution_engine::ExecutionEngine<'static>, String> {
    let types = Types::build(context, structs, program)?;
    let module = build_module(context, &types, program)?;
    optimize_module(&module)?;
    module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))
}

fn build_module<'ctx>(
    context: &'ctx Context,
    types: &Types<'ctx>,
    program: &Program,
) -> Result<inkwell::module::Module<'ctx>, String> {
    let module = context.create_module("zeta_native");
    let builder = context.create_builder();

    // libc malloc/memcpy for array buffers + deep copies (link via libc).
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let malloc = module.add_function("malloc", ptr_ty.fn_type(&[i64_ty.into()], false), None);
    let free = module.add_function(
        "free",
        context.void_type().fn_type(&[ptr_ty.into()], false),
        None,
    );
    let memcpy = module.add_function(
        "memcpy",
        ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
        None,
    );
    // libc memcmp for string equality (i32 memcmp(ptr, ptr, size_t)).
    let memcmp = module.add_function(
        "memcmp",
        context
            .i32_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
        None,
    );
    // libc snprintf for int_to_string (variadic: i32 snprintf(ptr, size_t, fmt, ...)).
    let snprintf = module.add_function(
        "snprintf",
        context
            .i32_type()
            .fn_type(&[ptr_ty.into(), i64_ty.into(), ptr_ty.into()], true),
        None,
    );

    // Generic functions can't be lowered as-is (LLVM is statically typed); they
    // are kept aside and monomorphized on demand at each call site.
    let generics: HashMap<String, &MirFunction> = program
        .functions
        .iter()
        .filter(|f| !f.type_params.is_empty())
        .map(|f| (f.name.clone(), f))
        .collect();
    let specialized: RefCell<HashMap<String, FunctionValue>> = RefCell::new(HashMap::new());

    // Pass 1: declare every concrete function with its typed signature.
    let mut functions: HashMap<String, FunctionValue> = HashMap::new();
    for function in &program.functions {
        if !function.type_params.is_empty() {
            continue;
        }
        let mut param_types = Vec::with_capacity(function.params.len());
        for param in &function.params {
            let zt = types.resolve_ann_ztype(&param.ty)?;
            param_types.push(types.llvm(&zt).into());
        }
        let ret = &types.returns[&function.name];
        let fn_type = types.llvm(ret).fn_type(&param_types, false);
        functions.insert(
            function.name.clone(),
            module.add_function(&function.name, fn_type, None),
        );
    }

    // Pass 2: lower each concrete body.
    for function in &program.functions {
        if !function.type_params.is_empty() {
            continue;
        }
        let llvm_fn = functions[&function.name];
        let entry_bb = context.append_basic_block(llvm_fn, "entry");
        builder.position_at_end(entry_bb);

        let mut lower = FnLower {
            context,
            module: &module,
            builder: &builder,
            types,
            functions: &functions,
            generics: &generics,
            specialized: &specialized,
            malloc,
            free,
            memcpy,
            memcmp,
            snprintf,
            llvm_fn,
            entry_bb,
            lambda_count: 0,
            locals: HashMap::new(),
            loops: Vec::new(),
        };
        // Parameters seed the top-level scope; further locals are allocated
        // on-demand as their `let` is lowered (see `lower_stmt`), with scope
        // save/restore around nested blocks so shadowed re-declarations of the
        // same name at different types get independent slots.
        for (index, param) in function.params.iter().enumerate() {
            let zt = types.resolve_ann_ztype(&param.ty)?;
            let slot = lower.entry_alloca(&param.name, types.llvm(&zt));
            let value = llvm_fn.get_nth_param(index as u32).expect("param exists");
            builder.build_store(slot, value).unwrap();
            lower.locals.insert(param.name.clone(), (slot, zt));
        }

        let terminated = lower
            .lower_stmts(&function.body)
            .map_err(|e| format!("in `{}`: {e}", function.name))?;
        if !terminated {
            let ret = &types.returns[&function.name];
            let zero = lower.zero_of(ret);
            builder.build_return(Some(&zero)).unwrap();
        }
    }

    module
        .verify()
        .map_err(|e| format!("LLVM module verification failed: {e}"))?;
    Ok(module)
}

fn host_target_machine() -> Result<inkwell::targets::TargetMachine, String> {
    use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("native target init failed: {e}"))?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| format!("target lookup failed: {e}"))?;
    target
        .create_target_machine(
            &triple,
            TargetMachine::get_host_cpu_name().to_str().unwrap_or(""),
            TargetMachine::get_host_cpu_features().to_str().unwrap_or(""),
            OptimizationLevel::Aggressive,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| "could not create host target machine".to_string())
}

fn optimize_module(module: &inkwell::module::Module) -> Result<(), String> {
    let machine = host_target_machine()?;
    module
        .run_passes(
            "default<O3>",
            &machine,
            inkwell::passes::PassBuilderOptions::create(),
        )
        .map_err(|e| format!("optimization passes failed: {e}"))
}

/// Ahead-of-time: compile `program` to a native **object file** at `path`. The
/// `entry` function (e.g. `main`) is renamed to `zeta_entry` so it won't clash
/// with the C `main` of the driver that links against this object. This is the
/// JIT-free path — `cc obj.o driver.c -o exe` yields a standalone binary, a step
/// toward dropping Stage0.
pub fn aot_compile_object(
    program: &Program,
    structs: &[StructDecl],
    entry: &str,
    path: &std::path::Path,
) -> Result<(), String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    let entry_fn = module
        .get_function(entry)
        .ok_or_else(|| format!("entry `{entry}` not found"))?;
    entry_fn.as_global_value().set_name("zeta_entry");
    optimize_module(&module)?;
    let machine = host_target_machine()?;
    machine
        .write_to_file(&module, inkwell::targets::FileType::Object, path)
        .map_err(|e| format!("object emission failed: {e}"))
}

struct FnLower<'a, 'ctx> {
    context: &'ctx Context,
    module: &'a inkwell::module::Module<'ctx>,
    builder: &'a Builder<'ctx>,
    types: &'a Types<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
    /// Generic function bodies, by name (excluded from the concrete passes).
    /// Calls to these are monomorphized on demand at the call site.
    generics: &'a HashMap<String, &'a MirFunction>,
    /// Cache of monomorphized instances: mangled name → lifted LLVM function.
    /// Shared (RefCell) across all `FnLower`s so each (generic, type args) pair
    /// is generated once and reused.
    specialized: &'a RefCell<HashMap<String, FunctionValue<'ctx>>>,
    malloc: FunctionValue<'ctx>,
    free: FunctionValue<'ctx>,
    memcpy: FunctionValue<'ctx>,
    memcmp: FunctionValue<'ctx>,
    snprintf: FunctionValue<'ctx>,
    llvm_fn: FunctionValue<'ctx>,
    entry_bb: BasicBlock<'ctx>,
    /// Monotonic counter for naming lambdas lifted out of this function.
    lambda_count: u32,
    /// local name → (alloca slot, type)
    locals: HashMap<String, (PointerValue<'ctx>, ZType)>,
    /// Enclosing loops as `(continue_target, exit)`. `break` jumps to `exit`;
    /// `continue` jumps to `continue_target` — which is the condition head for
    /// `while`, but the increment/step block for `for` loops (so `continue` still
    /// advances the counter / runs the step, matching the interpreter).
    loops: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
}

impl<'a, 'ctx> FnLower<'a, 'ctx> {
    fn i64t(&self) -> inkwell::types::IntType<'ctx> {
        self.context.i64_type()
    }

    fn zero_of(&self, zt: &ZType) -> BasicValueEnum<'ctx> {
        match zt {
            ZType::Int => self.i64t().const_zero().into(),
            ZType::Float => self.context.f64_type().const_zero().into(),
            ZType::Struct(name) => self.types.struct_llvm(name).const_zero().into(),
            ZType::Array(_) | ZType::Str => array_struct_type(self.context).const_zero().into(),
            ZType::Enum(_) => enum_struct_type(self.context).const_zero().into(),
            ZType::Tuple(_) => self.types.llvm(zt).const_zero(),
            ZType::Closure(_, _) => closure_struct_type(self.context).const_zero().into(),
        }
    }

    /// Apply value-semantics at a binding point: if `value` is an array, return a
    /// deep copy (fresh malloc'd buffer) so the new owner is independent; other
    /// types are already value types in LLVM and pass through. (String elements
    /// are themselves immutable, so copying their `{len,ptr}` is safe sharing.)
    fn bind_value(&self, value: BasicValueEnum<'ctx>, zt: &ZType) -> BasicValueEnum<'ctx> {
        if let ZType::Array(elem) = zt {
            self.deep_copy_array(value.into_struct_value(), elem).into()
        } else {
            value
        }
    }

    /// Like [`bind_value`] but skips the array deep-copy when `expr` already
    /// produced a fresh, unaliased buffer the binding can TAKE OWNERSHIP of —
    /// copying it would be pure waste (and would leak the original). Such sources:
    /// an array literal, an `*_array_*` builtin, or any function/closure call
    /// returning an array (the callee transfers ownership of the returned buffer;
    /// see the array-return path in `lower_stmt`). Strings are never copied
    /// regardless, being immutable.
    fn bind_owned(
        &self,
        expr: &MirExpr,
        value: BasicValueEnum<'ctx>,
        zt: &ZType,
    ) -> BasicValueEnum<'ctx> {
        let owns =
            is_fresh_array(expr) || (matches!(expr, MirExpr::Call { .. }) && matches!(zt, ZType::Array(_)));
        if owns {
            value
        } else {
            self.bind_value(value, zt)
        }
    }

    /// Byte size of one element of `elem` (8 for Int, 16 for the `{len,ptr}` of
    /// String/array elements, etc.) as a runtime i64 (LLVM folds it to a constant).
    fn elem_bytes(&self, elem: &ZType) -> IntValue<'ctx> {
        self.types.llvm(elem).size_of().unwrap()
    }

    /// Allocate an array heap buffer with an 8-byte capacity header:
    /// `[ i64 cap | cap * elem_size bytes ]`. Returns the ELEMENTS pointer (one
    /// header past the base), so the rest of codegen still sees a plain
    /// `{len, ptr}` where `ptr` points straight at element 0. `cap` is stored so
    /// in-place `push` can grow amortized-O(1) (see [`array_cap`]).
    fn alloc_array_buf(&self, cap: IntValue<'ctx>, elem_size: IntValue<'ctx>) -> PointerValue<'ctx> {
        let b = self.builder;
        let header = self.i64t().const_int(8, false);
        let elem_bytes = b.build_int_mul(cap, elem_size, "capbytes").unwrap();
        let total = b.build_int_add(header, elem_bytes, "totbytes").unwrap();
        let base = self.malloc_bytes(total);
        b.build_store(base, cap).unwrap();
        unsafe {
            b.build_in_bounds_gep(self.context.i8_type(), base, &[header], "elems")
                .unwrap()
        }
    }

    /// Read the capacity header stored 8 bytes before the elements pointer.
    fn array_cap(&self, elems: PointerValue<'ctx>) -> IntValue<'ctx> {
        let b = self.builder;
        let back = self.i64t().const_int((-8i64) as u64, true);
        let hdr = unsafe {
            b.build_in_bounds_gep(self.context.i8_type(), elems, &[back], "caphdr")
                .unwrap()
        };
        b.build_load(self.i64t(), hdr, "cap").unwrap().into_int_value()
    }

    /// Deep-copy an `{len, data}` array value into a fresh capacity-headed buffer
    /// (cap = len), memcpy the elements (stride = `elem`'s size), return `{len,
    /// newdata}`.
    fn deep_copy_array(
        &self,
        arr: inkwell::values::StructValue<'ctx>,
        elem: &ZType,
    ) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let len = b.build_extract_value(arr, 0, "len").unwrap().into_int_value();
        let src = b.build_extract_value(arr, 1, "data").unwrap().into_pointer_value();
        let elem_size = self.elem_bytes(elem);
        let bytes = b.build_int_mul(len, elem_size, "bytes").unwrap();
        let dst = self.alloc_array_buf(len, elem_size);
        b.build_call(self.memcpy, &[dst.into(), src.into(), bytes.into()], "cp")
            .unwrap();
        self.make_len_ptr(len, dst)
    }

    /// Allocate a slot of `ty` at the TOP of the entry block (mem2reg-friendly).
    fn entry_alloca(&self, name: &str, ty: BasicTypeEnum<'ctx>) -> PointerValue<'ctx> {
        let saved = self.builder.get_insert_block();
        match self.entry_bb.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(self.entry_bb),
        }
        let slot = self.builder.build_alloca(ty, name).unwrap();
        if let Some(block) = saved {
            self.builder.position_at_end(block);
        }
        slot
    }

    fn lower_stmts(&mut self, stmts: &[MirStmt]) -> Result<bool, String> {
        for stmt in stmts {
            if self.lower_stmt(stmt)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Free the heap buffers of array-typed locals declared since the `saved`
    /// scope snapshot. Each array local UNIQUELY owns its capacity-headed buffer
    /// (value semantics deep-copies the array at every other binding point), so
    /// on the fall-through path at scope exit the local is dead with no live
    /// alias — freeing it is sound. Strings (immutable, shared), closure
    /// environments, and boxed enum payloads are shared by value-copy and are
    /// NOT freed here. Called only on fall-through (never after a return / break /
    /// continue, where the value may escape and trailing code is unreachable).
    fn free_scope_locals(&mut self, saved: &HashMap<String, (PointerValue<'ctx>, ZType)>) {
        let mut slots: Vec<PointerValue<'ctx>> = Vec::new();
        for (name, (slot, zt)) in self.locals.iter() {
            if !matches!(zt, ZType::Array(_)) {
                continue;
            }
            // Declared in THIS scope = absent from the snapshot, or a shadow
            // re-using the name with a different slot. The outer binding (same
            // slot) is left for its own scope to free.
            let outer = saved.get(name).map(|(s, _)| *s == *slot).unwrap_or(false);
            if !outer {
                slots.push(*slot);
            }
        }
        for slot in slots {
            self.free_array_at_slot(slot);
        }
    }

    /// Emit `free` for the array buffer currently held in `slot`: load `{len,
    /// ptr}`, recover the malloc base (`ptr - 8` capacity header), free it.
    fn free_array_at_slot(&self, slot: PointerValue<'ctx>) {
        let arr = self
            .builder
            .build_load(array_struct_type(self.context), slot, "freearr")
            .unwrap()
            .into_struct_value();
        let data = self
            .builder
            .build_extract_value(arr, 1, "freedata")
            .unwrap()
            .into_pointer_value();
        let back = self.i64t().const_int((-8i64) as u64, true);
        let base = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[back], "freebase")
                .unwrap()
        };
        self.builder.build_call(self.free, &[base.into()], "").unwrap();
    }

    /// Free every currently-live array local except `skip` (the slot whose buffer
    /// is being returned, for an array return — its ownership transfers to the
    /// caller). One entry per name; the newest binding shadows. Sound because the
    /// function is exiting so every other array local is dead.
    fn free_live_arrays_except(&self, skip: Option<PointerValue<'ctx>>) {
        let slots: Vec<PointerValue<'ctx>> = self
            .locals
            .values()
            .filter(|(_, zt)| matches!(zt, ZType::Array(_)))
            .map(|(slot, _)| *slot)
            .filter(|slot| Some(*slot) != skip)
            .collect();
        for slot in slots {
            self.free_array_at_slot(slot);
        }
    }

    /// Lower a nested block in its own lexical scope: locals declared inside are
    /// discarded afterwards, so a sibling block may reuse a name at a different
    /// type (each `let` gets its own slot). Returns whether the block terminates.
    /// On the fall-through path, array locals declared in the block are freed so
    /// a loop body reclaims its per-iteration allocations.
    fn lower_block(&mut self, stmts: &[MirStmt]) -> Result<bool, String> {
        let saved = self.locals.clone();
        let terminated = self.lower_stmts(stmts)?;
        if !terminated {
            self.free_scope_locals(&saved);
        }
        self.locals = saved;
        Ok(terminated)
    }

    fn lower_stmt(&mut self, stmt: &MirStmt) -> Result<bool, String> {
        match stmt {
            MirStmt::Local { name, value, .. } => {
                // Lower the initializer FIRST (so `let x = x + 1` reads the outer
                // `x`), then allocate a fresh slot typed by the value and bind it —
                // shadowing any outer binding until this scope ends.
                let (v, vt) = self.lower_expr(value)?;
                let v = self.bind_owned(value, v, &vt);
                let slot = self.entry_alloca(name, self.types.llvm(&vt));
                self.builder.build_store(slot, v).unwrap();
                self.locals.insert(name.clone(), (slot, vt));
                Ok(false)
            }
            MirStmt::Store { place, value } => {
                // Peephole: `xs = <int|bool|string>_array_push(xs, v)` mutates xs's
                // buffer in place (amortized O(1)) instead of copying. Sound because
                // value semantics give every variable a uniquely-owned buffer, so no
                // other live owner observes the old buffer.
                if let Some((name, elem, value_arg)) = match_inplace_push(place, value) {
                    self.lower_inplace_push(name, value_arg, elem)?;
                    return Ok(false);
                }
                let (v, vt) = self.lower_expr(value)?;
                let v = self.bind_owned(value, v, &vt);
                let (slot, slot_ty) = self.resolve_place(place)?;
                // Reassigning a simple array local: free the old buffer first. The
                // new value was already deep-copied / freshly allocated by
                // `bind_owned`, so it cannot alias the old buffer, and the old
                // buffer is uniquely owned by this local — safe to free.
                if matches!(place, MirPlace::Local(_)) && matches!(slot_ty, ZType::Array(_)) {
                    self.free_array_at_slot(slot);
                }
                self.builder.build_store(slot, v).unwrap();
                Ok(false)
            }
            MirStmt::Return(value) => {
                let v = match value {
                    Some(expr) => {
                        let (v, vt) = self.lower_expr(expr)?;
                        if matches!(vt, ZType::Array(_)) {
                            // Array return: ownership of the returned buffer
                            // transfers to the caller (which takes it without a
                            // copy — see `bind_owned`). Free every OTHER array
                            // local; if returning a local directly, skip its slot
                            // (that buffer IS the return value).
                            let skip = match expr {
                                MirExpr::Load(name) => self.locals.get(name).and_then(|(slot, zt)| {
                                    matches!(zt, ZType::Array(_)).then_some(*slot)
                                }),
                                _ => None,
                            };
                            self.free_live_arrays_except(skip);
                        } else {
                            // Non-array return: no array escapes, free them all.
                            self.free_live_arrays_except(None);
                        }
                        v
                    }
                    None => {
                        self.free_live_arrays_except(None);
                        self.i64t().const_zero().into()
                    }
                };
                self.builder.build_return(Some(&v)).unwrap();
                Ok(true)
            }
            MirStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let cond = self.lower_cond(condition)?;
                let then_bb = self.context.append_basic_block(self.llvm_fn, "then");
                let else_bb = self.context.append_basic_block(self.llvm_fn, "else");
                let cont_bb = self.context.append_basic_block(self.llvm_fn, "endif");
                self.builder
                    .build_conditional_branch(cond, then_bb, else_bb)
                    .unwrap();
                self.builder.position_at_end(then_bb);
                if !self.lower_block(then_body)? {
                    self.builder.build_unconditional_branch(cont_bb).unwrap();
                }
                self.builder.position_at_end(else_bb);
                if !self.lower_block(else_body)? {
                    self.builder.build_unconditional_branch(cont_bb).unwrap();
                }
                self.builder.position_at_end(cont_bb);
                Ok(false)
            }
            MirStmt::While { condition, body } => {
                let head = self.context.append_basic_block(self.llvm_fn, "while.head");
                let body_bb = self.context.append_basic_block(self.llvm_fn, "while.body");
                let exit = self.context.append_basic_block(self.llvm_fn, "while.exit");
                self.builder.build_unconditional_branch(head).unwrap();
                self.builder.position_at_end(head);
                let cond = self.lower_cond(condition)?;
                self.builder
                    .build_conditional_branch(cond, body_bb, exit)
                    .unwrap();
                self.loops.push((head, exit));
                self.builder.position_at_end(body_bb);
                if !self.lower_block(body)? {
                    self.builder.build_unconditional_branch(head).unwrap();
                }
                self.loops.pop();
                self.builder.position_at_end(exit);
                Ok(false)
            }
            MirStmt::Break => {
                let (_, exit) = *self.loops.last().ok_or("`break` outside loop")?;
                self.builder.build_unconditional_branch(exit).unwrap();
                Ok(true)
            }
            MirStmt::Continue => {
                let (head, _) = *self.loops.last().ok_or("`continue` outside loop")?;
                self.builder.build_unconditional_branch(head).unwrap();
                Ok(true)
            }
            MirStmt::Drop(expr) => {
                self.lower_expr(expr)?;
                Ok(false)
            }
            MirStmt::Match { value, arms } => self.lower_match(value, arms),
            MirStmt::ForRange {
                binding,
                start,
                end,
                body,
            } => self.lower_for_range(binding, start, end, body),
            MirStmt::ForIn {
                binding,
                iterable,
                body,
            } => self.lower_for_in(binding, iterable, body),
            MirStmt::ForC {
                init,
                condition,
                step,
                body,
            } => self.lower_for_c(init, condition, step, body),
        }
    }

    /// `for i in start..end`: evaluate both bounds once, then `while i < end` with
    /// `i` incremented in the latch (so `continue` still advances). Exclusive end,
    /// matching the interpreter.
    fn lower_for_range(
        &mut self,
        binding: &str,
        start: &MirExpr,
        end: &MirExpr,
        body: &[MirStmt],
    ) -> Result<bool, String> {
        let start_v = self.lower_int(start)?;
        let end_v = self.lower_int(end)?;
        // The loop variable is scoped to the loop: allocate a fresh Int slot and
        // bind it, restoring the scope on exit.
        let scope = self.locals.clone();
        let slot = self.entry_alloca(binding, self.i64t().into());
        self.locals.insert(binding.to_string(), (slot, ZType::Int));
        self.builder.build_store(slot, start_v).unwrap();

        let head = self.context.append_basic_block(self.llvm_fn, "for.head");
        let body_bb = self.context.append_basic_block(self.llvm_fn, "for.body");
        let latch = self.context.append_basic_block(self.llvm_fn, "for.latch");
        let exit = self.context.append_basic_block(self.llvm_fn, "for.exit");

        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let i = self.builder.build_load(self.i64t(), slot, "i").unwrap().into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::SLT, i, end_v, "for.cmp")
            .unwrap();
        self.builder.build_conditional_branch(cond, body_bb, exit).unwrap();

        self.loops.push((latch, exit));
        self.builder.position_at_end(body_bb);
        if !self.lower_block(body)? {
            self.builder.build_unconditional_branch(latch).unwrap();
        }
        self.loops.pop();

        self.builder.position_at_end(latch);
        let i = self.builder.build_load(self.i64t(), slot, "i").unwrap().into_int_value();
        let next = self.builder.build_int_add(i, self.i64t().const_int(1, false), "inc").unwrap();
        self.builder.build_store(slot, next).unwrap();
        self.builder.build_unconditional_branch(head).unwrap();
        self.locals = scope;

        self.builder.position_at_end(exit);
        Ok(false)
    }

    /// `for x in array`: walk indices `0..len`, binding each element (any element
    /// type — Int or String).
    fn lower_for_in(
        &mut self,
        binding: &str,
        iterable: &MirExpr,
        body: &[MirStmt],
    ) -> Result<bool, String> {
        let (arr, arr_ty) = self.lower_expr(iterable)?;
        let ZType::Array(elem) = arr_ty else {
            return Err("for-in iterable must be an array".into());
        };
        let elem_llvm = self.types.llvm(&elem);
        let arr = arr.into_struct_value();
        let len = self.builder.build_extract_value(arr, 0, "len").unwrap().into_int_value();
        let data = self.builder.build_extract_value(arr, 1, "data").unwrap().into_pointer_value();

        let idx_slot = self.entry_alloca("for.idx", self.i64t().into());
        self.builder.build_store(idx_slot, self.i64t().const_zero()).unwrap();
        // The element binding is scoped to the loop.
        let scope = self.locals.clone();
        let binding_slot = self.entry_alloca(binding, elem_llvm);
        self.locals.insert(binding.to_string(), (binding_slot, (*elem).clone()));

        let head = self.context.append_basic_block(self.llvm_fn, "forin.head");
        let body_bb = self.context.append_basic_block(self.llvm_fn, "forin.body");
        let latch = self.context.append_basic_block(self.llvm_fn, "forin.latch");
        let exit = self.context.append_basic_block(self.llvm_fn, "forin.exit");

        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let idx = self.builder.build_load(self.i64t(), idx_slot, "idx").unwrap().into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::SLT, idx, len, "forin.cmp")
            .unwrap();
        self.builder.build_conditional_branch(cond, body_bb, exit).unwrap();

        self.loops.push((latch, exit));
        self.builder.position_at_end(body_bb);
        // Bind the current element, then lower the body.
        let elem_ptr = unsafe { self.builder.build_in_bounds_gep(elem_llvm, data, &[idx], "ep").unwrap() };
        let elem_val = self.builder.build_load(elem_llvm, elem_ptr, "elem").unwrap();
        self.builder.build_store(binding_slot, elem_val).unwrap();
        if !self.lower_block(body)? {
            self.builder.build_unconditional_branch(latch).unwrap();
        }
        self.loops.pop();

        self.builder.position_at_end(latch);
        let idx = self.builder.build_load(self.i64t(), idx_slot, "idx").unwrap().into_int_value();
        let next = self.builder.build_int_add(idx, self.i64t().const_int(1, false), "inc").unwrap();
        self.builder.build_store(idx_slot, next).unwrap();
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(exit);
        self.locals = scope;
        Ok(false)
    }

    /// `for (init; cond; step) { body }`: init once, then `loop { if !cond break;
    /// body; step }`. `continue` jumps to the step block, matching the interpreter.
    fn lower_for_c(
        &mut self,
        init: &MirStmt,
        condition: &MirExpr,
        step: &MirStmt,
        body: &[MirStmt],
    ) -> Result<bool, String> {
        // `init` declares the loop variable, scoped to the for; restore on exit.
        let scope = self.locals.clone();
        self.lower_stmt(init)?;

        let head = self.context.append_basic_block(self.llvm_fn, "forc.head");
        let body_bb = self.context.append_basic_block(self.llvm_fn, "forc.body");
        let step_bb = self.context.append_basic_block(self.llvm_fn, "forc.step");
        let exit = self.context.append_basic_block(self.llvm_fn, "forc.exit");

        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let cond = self.lower_cond(condition)?;
        self.builder.build_conditional_branch(cond, body_bb, exit).unwrap();

        self.loops.push((step_bb, exit));
        self.builder.position_at_end(body_bb);
        if !self.lower_block(body)? {
            self.builder.build_unconditional_branch(step_bb).unwrap();
        }
        self.loops.pop();

        self.builder.position_at_end(step_bb);
        self.lower_stmt(step)?;
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(exit);
        self.locals = scope;
        Ok(false)
    }

    /// Lower a `match`: switch on a single i64 scrutinee (an enum's tag, or an
    /// Int/Bool value itself). Each concrete pattern becomes a switch case; a
    /// `Name`/`Wildcard` arm is the default. Exhaustiveness is guaranteed by the
    /// MIR verifier, so when there is no catch-all the switch default is
    /// `unreachable`. Returns whether control is guaranteed terminated afterwards.
    fn lower_match(&mut self, value: &MirExpr, arms: &[crate::mir::MirMatchArm]) -> Result<bool, String> {
        let (val, vty) = self.lower_expr(value)?;
        let scrutinee = match &vty {
            ZType::Enum(_) => self
                .builder
                .build_extract_value(val.into_struct_value(), 0, "tag")
                .unwrap()
                .into_int_value(),
            ZType::Int => val.into_int_value(),
            _ => return Err("match scrutinee must be an enum or Int/Bool".into()),
        };
        // The scrutinee's enum instance name (mangled for a generic instance like
        // `Option$Int`) — payload types come from its monomorphized variant table,
        // not the pattern's base name.
        let enum_instance: Option<String> = match &vty {
            ZType::Enum(name) => Some(name.clone()),
            _ => None,
        };
        // The block holding the scrutinee; the switch terminates it. Building the
        // `unreachable` default below repositions the builder, so capture it now.
        let head_bb = self.builder.get_insert_block().unwrap();

        let arm_blocks: Vec<BasicBlock<'ctx>> = arms
            .iter()
            .map(|_| self.context.append_basic_block(self.llvm_fn, "arm"))
            .collect();
        let end_bb = self.context.append_basic_block(self.llvm_fn, "match.end");

        // Map each concrete pattern to (case const, its arm block); find the
        // catch-all arm (first Name/Wildcard) to use as the switch default.
        let mut cases: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut default_bb: Option<BasicBlock<'ctx>> = None;
        for (i, arm) in arms.iter().enumerate() {
            match &arm.pattern {
                MirPattern::Name(_) | MirPattern::Wildcard => {
                    if default_bb.is_none() {
                        default_bb = Some(arm_blocks[i]);
                    }
                }
                MirPattern::Variant { enum_name, variant, .. } => {
                    let tag = self.types.variant_index(enum_name, variant)?;
                    cases.push((self.i64t().const_int(tag, false), arm_blocks[i]));
                }
                MirPattern::Int(text) => {
                    let n: i64 = text.parse().map_err(|_| format!("bad Int pattern `{text}`"))?;
                    cases.push((self.i64t().const_int(n as u64, true), arm_blocks[i]));
                }
                MirPattern::Bool(b) => {
                    cases.push((self.i64t().const_int(*b as u64, false), arm_blocks[i]));
                }
                MirPattern::String(_) => {
                    return Err("string match patterns not in the native subset".into())
                }
            }
        }

        // Exhaustive-but-no-catch-all → an `unreachable` default block.
        let default = match default_bb {
            Some(bb) => bb,
            None => {
                let bb = self.context.append_basic_block(self.llvm_fn, "match.unreachable");
                self.builder.position_at_end(bb);
                self.builder.build_unreachable().unwrap();
                bb
            }
        };
        self.builder.position_at_end(head_bb);
        self.builder.build_switch(scrutinee, default, &cases).unwrap();

        // Lower each arm in its own scope: allocate + bind its pattern variable
        // (if any), lower the body, then restore the scope.
        for (i, arm) in arms.iter().enumerate() {
            self.builder.position_at_end(arm_blocks[i]);
            let scope = self.locals.clone();
            match &arm.pattern {
                MirPattern::Name(name) => {
                    let slot = self.entry_alloca(name, self.types.llvm(&vty));
                    self.builder.build_store(slot, val).unwrap();
                    self.locals.insert(name.clone(), (slot, vty.clone()));
                }
                MirPattern::Variant {
                    enum_name,
                    variant,
                    binding: Some(binding),
                } => {
                    let lookup = enum_instance.as_deref().unwrap_or(enum_name);
                    let (_, payload_ty) = self.types.variant_tag(lookup, variant)?;
                    let sv = val.into_struct_value();
                    let (bound, bty) = match payload_ty {
                        Some(ZType::Int) => {
                            // p0 holds the Int payload.
                            let p0 = self.builder.build_extract_value(sv, 1, "payload").unwrap();
                            (p0, ZType::Int)
                        }
                        Some(ZType::Str) => {
                            // Reconstruct the String {len, ptr} from (p0, p1).
                            let p0 = self.builder.build_extract_value(sv, 1, "plen").unwrap().into_int_value();
                            let p1 = self.builder.build_extract_value(sv, 2, "pdata").unwrap().into_pointer_value();
                            (self.make_len_ptr(p0, p1).into(), ZType::Str)
                        }
                        Some(ZType::Array(elem)) => {
                            // Rebuild {len, ptr} from (p0, p1), then deep-copy so the
                            // bound local is an independent owner (safe to push/mutate).
                            let p0 = self.builder.build_extract_value(sv, 1, "plen").unwrap().into_int_value();
                            let p1 = self.builder.build_extract_value(sv, 2, "pdata").unwrap().into_pointer_value();
                            let owned = self.deep_copy_array(self.make_len_ptr(p0, p1), &elem);
                            (owned.into(), ZType::Array(elem))
                        }
                        Some(ZType::Struct(name)) => {
                            // p1 points at the boxed struct; load it back by value.
                            let p1 = self.builder.build_extract_value(sv, 2, "pbox").unwrap().into_pointer_value();
                            let struct_ty = self.types.struct_llvm(&name);
                            let loaded = self.builder.build_load(struct_ty, p1, "boxload").unwrap();
                            (loaded, ZType::Struct(name))
                        }
                        _ => return Err("enum payload type not in the native subset".into()),
                    };
                    let slot = self.entry_alloca(binding, self.types.llvm(&bty));
                    self.builder.build_store(slot, bound).unwrap();
                    self.locals.insert(binding.clone(), (slot, bty));
                }
                _ => {}
            }
            if !self.lower_stmts(&arm.body)? {
                self.builder.build_unconditional_branch(end_bb).unwrap();
            }
            self.locals = scope;
        }

        self.builder.position_at_end(end_bb);
        Ok(false)
    }

    /// Resolve an assignment place to (pointer-to-slot, type).
    fn resolve_place(&mut self, place: &MirPlace) -> Result<(PointerValue<'ctx>, ZType), String> {
        match place {
            MirPlace::Local(name) => {
                let (slot, zt) = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("store to unknown local `{name}`"))?;
                Ok((*slot, zt.clone()))
            }
            MirPlace::Field { base, field } => {
                let (base_ptr, base_ty) = self.resolve_place(base)?;
                let ZType::Struct(struct_name) = base_ty else {
                    return Err("field assignment on non-struct".into());
                };
                let (index, field_ty) = self.types.field_index(&struct_name, field)?;
                let struct_ty = self.types.struct_llvm(&struct_name);
                let field_ptr = self
                    .builder
                    .build_struct_gep(struct_ty, base_ptr, index, "fieldptr")
                    .map_err(|_| "struct GEP failed".to_string())?;
                Ok((field_ptr, field_ty))
            }
            MirPlace::Index { base, index } => {
                let (base_slot, base_ty) = self.resolve_place(base)?;
                let ZType::Array(elem) = base_ty else {
                    return Err("index assignment on non-array".into());
                };
                // Load the {len, data} struct from the base slot, GEP into the
                // (exclusively owned) heap buffer, and return the element ptr.
                let arr = self
                    .builder
                    .build_load(array_struct_type(self.context), base_slot, "arr")
                    .unwrap()
                    .into_struct_value();
                let data = self
                    .builder
                    .build_extract_value(arr, 1, "data")
                    .unwrap()
                    .into_pointer_value();
                let idx = self.lower_int(index)?;
                let elem_llvm = self.types.llvm(&elem);
                let elem_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(elem_llvm, data, &[idx], "elemptr")
                        .unwrap()
                };
                Ok((elem_ptr, *elem))
            }
        }
    }

    /// Infer the `ZType` an expression lowers to, WITHOUT emitting code, given the
    /// types of the locals in scope. Used to learn a lambda body's return type so
    /// the lifted function can be created with the right signature. Mirrors the
    /// type each `lower_expr` arm produces; unsupported forms in a closure body
    /// error out (the interpreter remains the oracle for those).
    fn infer_ztype(
        &self,
        expr: &MirExpr,
        local_types: &HashMap<String, ZType>,
    ) -> Result<ZType, String> {
        match expr {
            MirExpr::Int(_) | MirExpr::Bool(_) => Ok(ZType::Int),
            MirExpr::Float(_) => Ok(ZType::Float),
            MirExpr::String(_) => Ok(ZType::Str),
            MirExpr::Load(name) => local_types
                .get(name)
                .cloned()
                .ok_or_else(|| format!("closure body loads unknown local `{name}`")),
            MirExpr::Unary { op, expr } => {
                let inner = self.infer_ztype(expr, local_types)?;
                match op {
                    UnaryOp::Neg if inner == ZType::Float => Ok(ZType::Float),
                    _ => Ok(ZType::Int),
                }
            }
            MirExpr::Binary { op, left, right } => match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    let l = self.infer_ztype(left, local_types)?;
                    let r = self.infer_ztype(right, local_types)?;
                    if l == ZType::Float || r == ZType::Float {
                        Ok(ZType::Float)
                    } else {
                        Ok(ZType::Int)
                    }
                }
                // Mod, bitwise, comparisons and logical ops all yield i64.
                _ => Ok(ZType::Int),
            },
            MirExpr::Call { callee, args } => {
                if let Some(ret) = self.types.returns.get(callee) {
                    return Ok(ret.clone());
                }
                // Indirect call through a closure-typed local: its return type.
                if let Some(ZType::Closure(_, ret)) = local_types.get(callee) {
                    return Ok((**ret).clone());
                }
                match callee.as_str() {
                    "string_len" | "string_byte_at" => Ok(ZType::Int),
                    "string_concat" | "string_byte_slice" | "int_to_string" => Ok(ZType::Str),
                    "ascii_is_digit" | "ascii_is_alpha" | "ascii_is_alnum"
                    | "ascii_is_whitespace" => Ok(ZType::Int),
                    "int_array_empty" | "int_array_push" | "bool_array_empty"
                    | "bool_array_push" => Ok(ZType::Array(Box::new(ZType::Int))),
                    "string_array_empty" | "string_array_push" => {
                        Ok(ZType::Array(Box::new(ZType::Str)))
                    }
                    "float_array_empty" | "float_array_push" => {
                        Ok(ZType::Array(Box::new(ZType::Float)))
                    }
                    _ => {
                        let _ = args;
                        Err(format!(
                            "call `{callee}` in a closure body is not in the native subset"
                        ))
                    }
                }
            }
            MirExpr::Tuple { elements } => {
                let types = elements
                    .iter()
                    .map(|e| self.infer_ztype(e, local_types))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ZType::Tuple(types))
            }
            MirExpr::FieldAccess { base, field } => {
                let base_ty = self.infer_ztype(base, local_types)?;
                match base_ty {
                    ZType::Struct(name) => Ok(self.types.field_index(&name, field)?.1),
                    ZType::Tuple(elems) => {
                        let idx: usize = field
                            .parse()
                            .map_err(|_| format!("bad tuple index `.{field}`"))?;
                        elems
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| format!("tuple index `.{field}` out of range"))
                    }
                    ZType::Array(_) if field == "len" => Ok(ZType::Int),
                    other => Err(format!("field `.{field}` on `{other:?}` not supported")),
                }
            }
            MirExpr::Index { base, .. } => {
                let base_ty = self.infer_ztype(base, local_types)?;
                match base_ty {
                    ZType::Array(elem) => Ok(*elem),
                    other => Err(format!("index of `{other:?}` not supported")),
                }
            }
            MirExpr::Lambda { params, body } => {
                let mut inner = local_types.clone();
                let mut ptys = Vec::with_capacity(params.len());
                for p in params {
                    let zt = self.types.resolve_ann_ztype(&p.ty)?;
                    inner.insert(p.name.clone(), zt.clone());
                    ptys.push(zt);
                }
                let ret = self.infer_ztype(body, &inner)?;
                Ok(ZType::Closure(ptys, Box::new(ret)))
            }
            MirExpr::StructLiteral { ty, .. } => Ok(ZType::Struct(ty.clone())),
            MirExpr::EnumVariant { enum_name, .. } => Ok(ZType::Enum(enum_name.clone())),
            MirExpr::ArrayLiteral { elements } => {
                let elem = match elements.first() {
                    Some(e) => self.infer_ztype(e, local_types)?,
                    None => ZType::Int,
                };
                Ok(ZType::Array(Box::new(elem)))
            }
        }
    }

    /// Closure conversion: lift `|params| body` to a top-level LLVM function whose
    /// first parameter is a heap-allocated environment of captured variables, then
    /// build the closure value `{ fn_ptr, env_ptr }` at the use site. Captured
    /// variables are copied into the environment BY VALUE at creation time,
    /// matching the interpreter's snapshot semantics.
    fn lower_lambda(
        &mut self,
        params: &[crate::ast::Param],
        body: &MirExpr,
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());

        // Parameter types from annotations.
        let param_ztys: Vec<ZType> = params
            .iter()
            .map(|p| self.types.resolve_ann_ztype(&p.ty))
            .collect::<Result<_, _>>()?;

        // Free variables that are enclosing locals → captured into the env.
        let mut bound: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut free = Vec::new();
        collect_free_loads(body, &mut bound, &mut free);
        let captures: Vec<(String, ZType, PointerValue<'ctx>)> = free
            .iter()
            .filter_map(|name| {
                self.locals
                    .get(name)
                    .map(|(slot, zt)| (name.clone(), zt.clone(), *slot))
            })
            .collect();

        // Infer the body's return type (for the lifted function's signature).
        let mut body_types: HashMap<String, ZType> = HashMap::new();
        for (name, zt, _) in &captures {
            body_types.insert(name.clone(), zt.clone());
        }
        for (param, zt) in params.iter().zip(&param_ztys) {
            body_types.insert(param.name.clone(), zt.clone());
        }
        let ret_zty = self.infer_ztype(body, &body_types)?;

        // Environment struct: one field per capture, in capture order.
        let env_field_types: Vec<BasicTypeEnum<'ctx>> =
            captures.iter().map(|(_, zt, _)| self.types.llvm(zt)).collect();
        let env_ty = self.context.struct_type(&env_field_types, false);

        // Lifted function signature: (env_ptr, params...) -> ret.
        let mut fn_param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            vec![ptr_ty.into()];
        for zt in &param_ztys {
            fn_param_types.push(self.types.llvm(zt).into());
        }
        let fn_type = self.types.llvm(&ret_zty).fn_type(&fn_param_types, false);
        let name = format!(
            "__lambda_{}_{}",
            self.llvm_fn.get_name().to_str().unwrap_or("fn"),
            self.lambda_count
        );
        self.lambda_count += 1;
        let lifted = self.module.add_function(&name, fn_type, None);

        // Lower the body into the lifted function, then restore the outer builder.
        let saved_block = self.builder.get_insert_block();
        let lifted_entry = self.context.append_basic_block(lifted, "entry");
        self.builder.position_at_end(lifted_entry);
        {
            let mut inner = FnLower {
                context: self.context,
                module: self.module,
                builder: self.builder,
                types: self.types,
                functions: self.functions,
                generics: self.generics,
                specialized: self.specialized,
                malloc: self.malloc,
                free: self.free,
                memcpy: self.memcpy,
                memcmp: self.memcmp,
                snprintf: self.snprintf,
                llvm_fn: lifted,
                entry_bb: lifted_entry,
                lambda_count: 0,
                locals: HashMap::new(),
                loops: Vec::new(),
            };
            let env_ptr = lifted
                .get_nth_param(0)
                .expect("env param exists")
                .into_pointer_value();
            for (index, (cap_name, cap_ty, _)) in captures.iter().enumerate() {
                let llvm_ty = inner.types.llvm(cap_ty);
                let field_ptr = inner
                    .builder
                    .build_struct_gep(env_ty, env_ptr, index as u32, "cap")
                    .map_err(|_| "env GEP failed".to_string())?;
                let val = inner.builder.build_load(llvm_ty, field_ptr, cap_name).unwrap();
                let slot = inner.entry_alloca(cap_name, llvm_ty);
                inner.builder.build_store(slot, val).unwrap();
                inner.locals.insert(cap_name.clone(), (slot, cap_ty.clone()));
            }
            for (index, (param, zt)) in params.iter().zip(&param_ztys).enumerate() {
                let llvm_ty = inner.types.llvm(zt);
                let slot = inner.entry_alloca(&param.name, llvm_ty);
                let value = lifted
                    .get_nth_param((index + 1) as u32)
                    .expect("lambda param exists");
                inner.builder.build_store(slot, value).unwrap();
                inner.locals.insert(param.name.clone(), (slot, zt.clone()));
            }
            let (rv, _) = inner.lower_expr(body)?;
            inner.builder.build_return(Some(&rv)).unwrap();
        }
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        // Allocate the environment and copy captured values into it by value.
        let env_size = env_ty
            .size_of()
            .ok_or_else(|| "env type has no size".to_string())?;
        let env_mem = self.malloc_bytes(env_size);
        for (index, (cap_name, cap_ty, slot)) in captures.iter().enumerate() {
            let llvm_ty = self.types.llvm(cap_ty);
            let val = self.builder.build_load(llvm_ty, *slot, cap_name).unwrap();
            let field_ptr = self
                .builder
                .build_struct_gep(env_ty, env_mem, index as u32, "capst")
                .map_err(|_| "env store GEP failed".to_string())?;
            self.builder.build_store(field_ptr, val).unwrap();
        }

        // Build the closure value { fn_ptr, env_ptr }.
        let clo_ty = closure_struct_type(self.context);
        let fn_ptr = lifted.as_global_value().as_pointer_value();
        let clo = self
            .builder
            .build_insert_value(clo_ty.get_undef(), fn_ptr, 0, "clo_fn")
            .unwrap();
        let clo = self
            .builder
            .build_insert_value(clo, env_mem, 1, "clo_env")
            .unwrap()
            .into_struct_value();
        Ok((clo.into(), ZType::Closure(param_ztys, Box::new(ret_zty))))
    }

    /// Call a closure held in local `slot`: load `{ fn_ptr, env_ptr }`, then invoke
    /// `fn_ptr(env_ptr, args...)` with the signature rebuilt from the closure type.
    fn lower_indirect_call(
        &mut self,
        slot: PointerValue<'ctx>,
        param_ztys: &[ZType],
        ret_zty: &ZType,
        args: &[MirExpr],
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let clo_ty = closure_struct_type(self.context);
        let clo = self
            .builder
            .build_load(clo_ty, slot, "clo")
            .unwrap()
            .into_struct_value();
        let fn_ptr = self
            .builder
            .build_extract_value(clo, 0, "clo_fn")
            .unwrap()
            .into_pointer_value();
        let env_ptr = self
            .builder
            .build_extract_value(clo, 1, "clo_env")
            .unwrap()
            .into_pointer_value();

        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let mut fn_param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            vec![ptr_ty.into()];
        for zt in param_ztys {
            fn_param_types.push(self.types.llvm(zt).into());
        }
        let fn_type = self.types.llvm(ret_zty).fn_type(&fn_param_types, false);

        let mut argv: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = vec![env_ptr.into()];
        for arg in args {
            let (v, vt) = self.lower_expr(arg)?;
            argv.push(self.bind_owned(arg, v, &vt).into());
        }
        let call = self
            .builder
            .build_indirect_call(fn_type, fn_ptr, &argv, "iclo")
            .unwrap();
        let value = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "closure call returned no value".to_string())?;
        Ok((value, ret_zty.clone()))
    }

    /// Monomorphize a call to a generic function: lower the arguments (their
    /// ZTypes ARE the concrete parameter types), derive the type-parameter
    /// substitution to resolve the return type, generate (or reuse) the
    /// specialized instance, and call it.
    fn lower_generic_call(
        &mut self,
        callee: &str,
        args: &[MirExpr],
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let generic = *self
            .generics
            .get(callee)
            .ok_or_else(|| format!("unknown generic `{callee}`"))?;
        if generic.params.len() != args.len() {
            return Err(format!("generic `{callee}` arity mismatch"));
        }
        let mut argv: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(args.len());
        let mut arg_ztys = Vec::with_capacity(args.len());
        for arg in args {
            let (v, vt) = self.lower_expr(arg)?;
            let v = self.bind_owned(arg, v, &vt);
            argv.push(v.into());
            arg_ztys.push(vt);
        }
        let mut subst: HashMap<String, ZType> = HashMap::new();
        for (param, arg_zty) in generic.params.iter().zip(&arg_ztys) {
            unify_ztype(&param.ty, arg_zty, &generic.type_params, &mut subst);
        }
        let ret_zty = match &generic.return_type {
            Some(t) => self.resolve_generic_ztype(t, &subst)?,
            None => ZType::Int,
        };
        let mangled = mangle_instance(callee, &arg_ztys);
        let function = self.get_or_build_specialization(&mangled, generic, &arg_ztys, &ret_zty)?;
        let call = self.builder.build_call(function, &argv, "gcall").unwrap();
        let value = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{callee}` returned no value"))?;
        Ok((value, ret_zty))
    }

    /// Resolve a (possibly generic) declared type string to a concrete `ZType`,
    /// substituting bound type parameters.
    fn resolve_generic_ztype(
        &self,
        ty_str: &str,
        subst: &HashMap<String, ZType>,
    ) -> Result<ZType, String> {
        if let Some((params, ret)) = crate::type_syntax::fn_parts(ty_str) {
            let ps = params
                .iter()
                .map(|p| self.resolve_generic_ztype(p, subst))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ZType::Closure(
                ps,
                Box::new(self.resolve_generic_ztype(ret, subst)?),
            ));
        }
        if let Some(parts) = crate::type_syntax::tuple_parts(ty_str) {
            let es = parts
                .iter()
                .map(|p| self.resolve_generic_ztype(p, subst))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ZType::Tuple(es));
        }
        if let Some(zt) = subst.get(ty_str) {
            return Ok(zt.clone());
        }
        // A generic aggregate instantiation (`Box<T>` / `Option<Int>`): substitute
        // its arguments and monomorphize the concrete instance.
        if let Some((base, arg_strs)) = crate::type_syntax::generic_parts(ty_str) {
            if self.types.is_generic_struct(base) || self.types.is_generic_enum(base) {
                let args = arg_strs
                    .iter()
                    .map(|a| self.resolve_generic_ztype(a, subst))
                    .collect::<Result<Vec<_>, _>>()?;
                if self.types.is_generic_struct(base) {
                    return Ok(ZType::Struct(self.types.instantiate_struct(base, &args)?));
                }
                return Ok(ZType::Enum(self.types.instantiate_enum(base, &args)?));
            }
        }
        self.types.resolve_ann_ztype(ty_str)
    }

    /// Get or generate the monomorphized instance `mangled` of `generic` with the
    /// given concrete parameter / return types. Inserted into the shared cache
    /// before its body is lowered so self-recursive generics terminate.
    fn get_or_build_specialization(
        &self,
        mangled: &str,
        generic: &MirFunction,
        param_ztys: &[ZType],
        ret_zty: &ZType,
    ) -> Result<FunctionValue<'ctx>, String> {
        if let Some(func) = self.specialized.borrow().get(mangled) {
            return Ok(*func);
        }
        let mut fn_param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(param_ztys.len());
        for zt in param_ztys {
            fn_param_types.push(self.types.llvm(zt).into());
        }
        let fn_type = self.types.llvm(ret_zty).fn_type(&fn_param_types, false);
        let func = self.module.add_function(mangled, fn_type, None);
        self.specialized
            .borrow_mut()
            .insert(mangled.to_string(), func);

        let saved_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        {
            let mut inner = FnLower {
                context: self.context,
                module: self.module,
                builder: self.builder,
                types: self.types,
                functions: self.functions,
                generics: self.generics,
                specialized: self.specialized,
                malloc: self.malloc,
                free: self.free,
                memcpy: self.memcpy,
                memcmp: self.memcmp,
                snprintf: self.snprintf,
                llvm_fn: func,
                entry_bb: entry,
                lambda_count: 0,
                locals: HashMap::new(),
                loops: Vec::new(),
            };
            for (index, (param, zt)) in generic.params.iter().zip(param_ztys).enumerate() {
                let slot = inner.entry_alloca(&param.name, inner.types.llvm(zt));
                let value = func.get_nth_param(index as u32).expect("param exists");
                inner.builder.build_store(slot, value).unwrap();
                inner.locals.insert(param.name.clone(), (slot, zt.clone()));
            }
            let terminated = inner
                .lower_stmts(&generic.body)
                .map_err(|e| format!("in `{mangled}`: {e}"))?;
            if !terminated {
                let zero = inner.zero_of(ret_zty);
                inner.builder.build_return(Some(&zero)).unwrap();
            }
        }
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(func)
    }

    /// Lower an expression to (value, type).
    fn lower_expr(&mut self, expr: &MirExpr) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        match expr {
            MirExpr::Int(text) => {
                let n: i64 = text.parse().map_err(|_| format!("bad Int `{text}`"))?;
                Ok((self.i64t().const_int(n as u64, true).into(), ZType::Int))
            }
            MirExpr::Float(text) => {
                let n: f64 = text.parse().map_err(|_| format!("bad Float `{text}`"))?;
                Ok((self.context.f64_type().const_float(n).into(), ZType::Float))
            }
            MirExpr::Bool(b) => Ok((self.i64t().const_int(*b as u64, false).into(), ZType::Int)),
            MirExpr::Load(name) => {
                let (slot, zt) = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("load of unknown local `{name}`"))?;
                let llvm_ty = self.types.llvm(zt);
                let value = self.builder.build_load(llvm_ty, *slot, name).unwrap();
                Ok((value, zt.clone()))
            }
            MirExpr::Unary { op, expr } => {
                let (val, ty) = self.lower_expr(expr)?;
                if matches!(op, UnaryOp::Neg) && ty == ZType::Float {
                    let f = self.builder.build_float_neg(val.into_float_value(), "fneg").unwrap();
                    return Ok((f.into(), ZType::Float));
                }
                let v = val.into_int_value();
                let r = match op {
                    UnaryOp::Neg => self.builder.build_int_neg(v, "neg").unwrap(),
                    UnaryOp::BitNot => self.builder.build_not(v, "bitnot").unwrap(),
                    UnaryOp::Not => {
                        let z = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, v, self.i64t().const_zero(), "isz")
                            .unwrap();
                        self.builder.build_int_z_extend(z, self.i64t(), "not").unwrap()
                    }
                };
                Ok((r.into(), ZType::Int))
            }
            MirExpr::Binary { op, left, right } => self.lower_binary(*op, left, right),
            MirExpr::Call { callee, args } => {
                if let Some(result) = self.lower_builtin(callee, args)? {
                    return Ok(result);
                }
                if let Some(function) = self.functions.get(callee).copied() {
                    let mut argv = Vec::with_capacity(args.len());
                    for arg in args {
                        let (v, vt) = self.lower_expr(arg)?;
                        argv.push(self.bind_owned(arg, v, &vt).into());
                    }
                    let call = self.builder.build_call(function, &argv, "call").unwrap();
                    let ret = self
                        .types
                        .returns
                        .get(callee)
                        .cloned()
                        .unwrap_or(ZType::Int);
                    let value = call
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| format!("`{callee}` returned no value"))?;
                    return Ok((value, ret));
                }
                // Generic call: monomorphize `callee` for the concrete argument
                // types at this site, then call the specialized instance.
                if self.generics.contains_key(callee) {
                    return self.lower_generic_call(callee, args);
                }
                // Indirect call: `callee` is a local holding a closure value.
                if let Some((slot, ZType::Closure(param_ztys, ret_zty))) =
                    self.locals.get(callee).cloned()
                {
                    return self.lower_indirect_call(slot, &param_ztys, &ret_zty, args);
                }
                Err(format!("call to unknown `{callee}`"))
            }
            MirExpr::StructLiteral { ty, fields } => {
                // Generic struct: lower fields in declaration order, infer the
                // type arguments from the field value types, then monomorphize a
                // concrete `Box$Int`-style instance.
                if self.types.is_generic_struct(ty) {
                    let (type_params, tmpl_fields) = self.types.struct_template_of(ty);
                    let mut lowered: Vec<BasicValueEnum<'ctx>> =
                        Vec::with_capacity(tmpl_fields.len());
                    let mut subst: HashMap<String, ZType> = HashMap::new();
                    for (fname, fty) in &tmpl_fields {
                        let value_expr = &fields
                            .iter()
                            .find(|f| &f.name == fname)
                            .ok_or_else(|| format!("missing field `{fname}` in `{ty}` literal"))?
                            .value;
                        let (v, vt) = self.lower_expr(value_expr)?;
                        unify_ztype(fty, &vt, &type_params, &mut subst);
                        lowered.push(v);
                    }
                    let args: Vec<ZType> = type_params
                        .iter()
                        .map(|p| subst.get(p).cloned().unwrap_or(ZType::Int))
                        .collect();
                    let mangled = self.types.instantiate_struct(ty, &args)?;
                    let struct_ty = self.types.struct_llvm(&mangled);
                    let mut current = struct_ty.get_undef();
                    for (index, v) in lowered.into_iter().enumerate() {
                        current = self
                            .builder
                            .build_insert_value(current, v, index as u32, "ins")
                            .unwrap()
                            .into_struct_value();
                    }
                    return Ok((current.into(), ZType::Struct(mangled)));
                }
                // Non-generic struct: lower field values in declaration order.
                let struct_ty = self.types.struct_llvm(ty);
                let field_order = self.types.struct_field_names(ty);
                let mut current = struct_ty.get_undef();
                for (index, field_name) in field_order.into_iter().enumerate() {
                    let value_expr = &fields
                        .iter()
                        .find(|f| f.name == field_name)
                        .ok_or_else(|| format!("missing field `{field_name}` in `{ty}` literal"))?
                        .value;
                    let (v, _) = self.lower_expr(value_expr)?;
                    current = self
                        .builder
                        .build_insert_value(current, v, index as u32, "ins")
                        .unwrap()
                        .into_struct_value();
                }
                Ok((current.into(), ZType::Struct(ty.clone())))
            }
            MirExpr::Tuple { elements } => {
                // Lower elements, learn each type, then build an anonymous struct.
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.lower_expr(element)?);
                }
                let elem_types: Vec<ZType> = values.iter().map(|(_, t)| t.clone()).collect();
                let tuple_ty = ZType::Tuple(elem_types);
                let struct_ty = self.types.llvm(&tuple_ty).into_struct_type();
                let mut current = struct_ty.get_undef();
                for (index, (v, _)) in values.into_iter().enumerate() {
                    current = self
                        .builder
                        .build_insert_value(current, v, index as u32, "tup")
                        .unwrap()
                        .into_struct_value();
                }
                Ok((current.into(), tuple_ty))
            }
            MirExpr::Lambda { params, body } => self.lower_lambda(params, body),
            MirExpr::FieldAccess { base, field } => {
                let (base_val, base_ty) = self.lower_expr(base)?;
                match base_ty {
                    ZType::Struct(struct_name) => {
                        let (index, field_ty) = self.types.field_index(&struct_name, field)?;
                        let value = self
                            .builder
                            .build_extract_value(base_val.into_struct_value(), index, "field")
                            .unwrap();
                        Ok((value, field_ty))
                    }
                    ZType::Tuple(elem_types) => {
                        let index: usize = field
                            .parse()
                            .map_err(|_| format!("invalid tuple index `.{field}`"))?;
                        let field_ty = elem_types
                            .get(index)
                            .ok_or_else(|| format!("tuple index `.{field}` out of range"))?
                            .clone();
                        let value = self
                            .builder
                            .build_extract_value(base_val.into_struct_value(), index as u32, "tup")
                            .unwrap();
                        Ok((value, field_ty))
                    }
                    ZType::Array(_) if field == "len" => {
                        let len = self
                            .builder
                            .build_extract_value(base_val.into_struct_value(), 0, "len")
                            .unwrap();
                        Ok((len, ZType::Int))
                    }
                    _ => Err(format!("field `{field}` access not in the native subset")),
                }
            }
            MirExpr::ArrayLiteral { elements } => {
                // Lower all elements first to learn the element type, then malloc a
                // buffer of `n * elem_size` and store each (stride = elem type).
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.lower_expr(element)?);
                }
                let elem = match values.first() {
                    Some((_, t)) => t.clone(),
                    None => ZType::Int,
                };
                let elem_llvm = self.types.llvm(&elem);
                let n = values.len();
                // Capacity-headed buffer (cap = n) so a later in-place push can grow.
                let data = self.alloc_array_buf(
                    self.i64t().const_int(n as u64, false),
                    self.elem_bytes(&elem),
                );
                for (i, (v, _)) in values.into_iter().enumerate() {
                    let ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                elem_llvm,
                                data,
                                &[self.i64t().const_int(i as u64, false)],
                                "ep",
                            )
                            .unwrap()
                    };
                    self.builder.build_store(ptr, v).unwrap();
                }
                let arr = self
                    .builder
                    .build_insert_value(
                        array_struct_type(self.context).get_undef(),
                        self.i64t().const_int(n as u64, false),
                        0,
                        "a0",
                    )
                    .unwrap();
                let arr = self
                    .builder
                    .build_insert_value(arr, data, 1, "a1")
                    .unwrap()
                    .into_struct_value();
                Ok((arr.into(), ZType::Array(Box::new(elem))))
            }
            MirExpr::Index { base, index } => {
                let (base_val, base_ty) = self.lower_expr(base)?;
                let ZType::Array(elem) = base_ty else {
                    return Err("index of non-array".into());
                };
                let elem_llvm = self.types.llvm(&elem);
                let data = self
                    .builder
                    .build_extract_value(base_val.into_struct_value(), 1, "data")
                    .unwrap()
                    .into_pointer_value();
                let idx = self.lower_int(index)?;
                let ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(elem_llvm, data, &[idx], "ep")
                        .unwrap()
                };
                let value = self.builder.build_load(elem_llvm, ptr, "elem").unwrap();
                Ok((value, *elem))
            }
            MirExpr::String(text) => {
                // Immutable bytes → a private global constant; the value is
                // `{ byte_len, ptr-to-global }`. `build_global_string_ptr` appends a
                // NUL, but `len` excludes it (matches the interpreter's byte count).
                let global = self.builder.build_global_string_ptr(text, "str").unwrap();
                let data = global.as_pointer_value();
                let len = self.i64t().const_int(text.len() as u64, false);
                Ok((self.make_len_ptr(len, data).into(), ZType::Str))
            }
            MirExpr::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // {tag, p0, p1}: tag = variant index. Payload goes into the generic
                // slot — Int/Bool in p0; String/array's {len, ptr} as (p0, p1); a
                // struct (too wide for the inline slot) is boxed on the heap with the
                // pointer in p1. No-payload leaves it zero/null. The encoding is
                // driven by the payload value's ACTUAL lowered type, so a generic
                // variant (`Some(T)`) needs no declared payload type — the type
                // argument is inferred here and the enum monomorphized accordingly.
                let tag = self.types.variant_index(enum_name, variant)?;
                let null = self.context.ptr_type(inkwell::AddressSpace::default()).const_null();
                let type_params = if self.types.is_generic_enum(enum_name) {
                    self.types.enum_template_params(enum_name)
                } else {
                    Vec::new()
                };
                let payload_decl = self.types.enum_variant_payload_str(enum_name, variant);
                let mut subst: HashMap<String, ZType> = HashMap::new();
                let (p0, p1) = match payload {
                    None => (self.i64t().const_zero(), null),
                    Some(expr) => {
                        let (v, vt) = self.lower_expr(expr)?;
                        if let Some(decl) = &payload_decl {
                            unify_ztype(decl, &vt, &type_params, &mut subst);
                        }
                        match &vt {
                            ZType::Int => (v.into_int_value(), null),
                            ZType::Str => {
                                let (len, data) = self.len_ptr_parts(v.into_struct_value());
                                (len, data)
                            }
                            ZType::Array(elem) => {
                                // Array payload reuses the String {len, ptr} split
                                // across (p0, p1). Deep-copy so the enum owns an
                                // independent buffer (value semantics).
                                let owned = self.deep_copy_array(v.into_struct_value(), elem);
                                let (len, data) = self.len_ptr_parts(owned);
                                (len, data)
                            }
                            ZType::Struct(name) => {
                                // Struct payload is wider than the inline slot: box a
                                // by-value copy on the heap, pointer in p1 (p0 unused).
                                let size = self.types.struct_llvm(name).size_of().unwrap();
                                let boxed = self.malloc_bytes(size);
                                self.builder.build_store(boxed, v).unwrap();
                                (self.i64t().const_zero(), boxed)
                            }
                            _ => return Err("enum payload type not in the native subset".into()),
                        }
                    }
                };
                let args: Vec<ZType> = type_params
                    .iter()
                    .map(|p| subst.get(p).cloned().unwrap_or(ZType::Int))
                    .collect();
                let mangled = self.types.instantiate_enum(enum_name, &args)?;
                let et = enum_struct_type(self.context);
                let ev = self.builder.build_insert_value(et.get_undef(), self.i64t().const_int(tag, false), 0, "e0").unwrap();
                let ev = self.builder.build_insert_value(ev, p0, 1, "e1").unwrap();
                let ev = self.builder.build_insert_value(ev, p1, 2, "e2").unwrap().into_struct_value();
                Ok((ev.into(), ZType::Enum(mangled)))
            }
        }
    }

    /// Lower a std builtin call, or `Ok(None)` if `callee` is not a builtin (then
    /// it is a user function). Strings are `{len, ptr<i8>}`; see `runtime.rs` for
    /// the differential-oracle semantics each builtin must match.
    fn lower_builtin(
        &mut self,
        callee: &str,
        args: &[MirExpr],
    ) -> Result<Option<(BasicValueEnum<'ctx>, ZType)>, String> {
        let b = self.builder;
        match callee {
            "string_len" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let len = b
                    .build_extract_value(s.into_struct_value(), 0, "slen")
                    .unwrap();
                Ok(Some((len, ZType::Int)))
            }
            "string_byte_at" => {
                // `data[index]` as an unsigned byte zero-extended to i64 (the
                // interpreter does `i64::from(u8)`). No bounds check, matching the
                // array path; tests only index in range.
                let (s, _) = self.lower_expr(&args[0])?;
                let data = b
                    .build_extract_value(s.into_struct_value(), 1, "sdata")
                    .unwrap()
                    .into_pointer_value();
                let idx = self.lower_int(&args[1])?;
                let i8t = self.context.i8_type();
                let ptr = unsafe { b.build_in_bounds_gep(i8t, data, &[idx], "bp").unwrap() };
                let byte = b.build_load(i8t, ptr, "byte").unwrap().into_int_value();
                let widened = b.build_int_z_extend(byte, self.i64t(), "byte64").unwrap();
                Ok(Some((widened.into(), ZType::Int)))
            }
            "string_concat" => {
                // malloc(la+lb), memcpy a then b, return {la+lb, buf}.
                let (a, _) = self.lower_expr(&args[0])?;
                let (bv, _) = self.lower_expr(&args[1])?;
                let (la, pa) = self.len_ptr_parts(a.into_struct_value());
                let (lb, pb) = self.len_ptr_parts(bv.into_struct_value());
                let total = b.build_int_add(la, lb, "clen").unwrap();
                let buf = self.malloc_bytes(total);
                self.memcpy_bytes(buf, pa, la);
                let i8t = self.context.i8_type();
                let tail = unsafe { b.build_in_bounds_gep(i8t, buf, &[la], "tail").unwrap() };
                self.memcpy_bytes(tail, pb, lb);
                Ok(Some((self.make_len_ptr(total, buf).into(), ZType::Str)))
            }
            "string_byte_slice" => {
                // s[start .. start+len] → malloc(len) + memcpy from data+start. No
                // bounds/utf-8 check (the array path is likewise unchecked); tests
                // stay in range.
                let (s, _) = self.lower_expr(&args[0])?;
                let (_, data) = self.len_ptr_parts(s.into_struct_value());
                let start = self.lower_int(&args[1])?;
                let len = self.lower_int(&args[2])?;
                let i8t = self.context.i8_type();
                let src = unsafe { b.build_in_bounds_gep(i8t, data, &[start], "slcsrc").unwrap() };
                let buf = self.malloc_bytes(len);
                self.memcpy_bytes(buf, src, len);
                Ok(Some((self.make_len_ptr(len, buf).into(), ZType::Str)))
            }
            "int_to_string" => {
                // snprintf(buf, 24, "%lld", n): 24 ≥ 20 digits + sign + NUL for any
                // i64. The i32 return is the byte length (excl. NUL), our `len`.
                let n = self.lower_int(&args[0])?;
                let fmt = b.build_global_string_ptr("%lld", "fmt").unwrap().as_pointer_value();
                let cap = self.i64t().const_int(24, false);
                let buf = self.malloc_bytes(cap);
                let written = b
                    .build_call(self.snprintf, &[buf.into(), cap.into(), fmt.into(), n.into()], "snp")
                    .unwrap()
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_int_value();
                let len = b.build_int_s_extend(written, self.i64t(), "len64").unwrap();
                Ok(Some((self.make_len_ptr(len, buf).into(), ZType::Str)))
            }
            // Growable arrays. bool arrays share the Int (i64) element repr; string
            // arrays carry `{len,ptr}` elements (stride from the element type).
            "int_array_empty" | "bool_array_empty" => Ok(Some(self.lower_array_empty(ZType::Int))),
            "string_array_empty" => Ok(Some(self.lower_array_empty(ZType::Str))),
            "float_array_empty" => Ok(Some(self.lower_array_empty(ZType::Float))),
            "int_array_push" | "bool_array_push" => {
                Ok(Some(self.lower_array_push(&args[0], &args[1], ZType::Int)?))
            }
            "string_array_push" => {
                Ok(Some(self.lower_array_push(&args[0], &args[1], ZType::Str)?))
            }
            "float_array_push" => {
                Ok(Some(self.lower_array_push(&args[0], &args[1], ZType::Float)?))
            }
            // ascii predicates: Int byte → Bool (i64 0/1). Out-of-[0,255] inputs
            // fall outside every range/equality, yielding 0 — matching the
            // interpreter's explicit `(0..=255)` guard.
            "ascii_is_digit" => {
                let v = self.lower_int(&args[0])?;
                let r = self.in_range(v, 48, 57);
                Ok(Some((self.bool_to_i64(r).into(), ZType::Int)))
            }
            "ascii_is_alpha" => {
                let v = self.lower_int(&args[0])?;
                let r = self.is_alpha(v);
                Ok(Some((self.bool_to_i64(r).into(), ZType::Int)))
            }
            "ascii_is_alnum" => {
                let v = self.lower_int(&args[0])?;
                let digit = self.in_range(v, 48, 57);
                let alpha = self.is_alpha(v);
                let r = b.build_or(digit, alpha, "alnum").unwrap();
                Ok(Some((self.bool_to_i64(r).into(), ZType::Int)))
            }
            "ascii_is_whitespace" => {
                // Rust is_ascii_whitespace: ' '(32) \t(9) \n(10) FF(12) \r(13);
                // note 0x0B (vertical tab) is excluded.
                let v = self.lower_int(&args[0])?;
                let mut acc = self.eq_const(v, 32);
                for k in [9, 10, 12, 13] {
                    let e = self.eq_const(v, k);
                    acc = b.build_or(acc, e, "ws").unwrap();
                }
                Ok(Some((self.bool_to_i64(acc).into(), ZType::Int)))
            }
            _ => Ok(None),
        }
    }

    /// `lo <= v <= hi` (signed) as an i1.
    fn in_range(&self, v: IntValue<'ctx>, lo: i64, hi: i64) -> IntValue<'ctx> {
        let b = self.builder;
        let ge = b
            .build_int_compare(IntPredicate::SGE, v, self.i64t().const_int(lo as u64, true), "ge")
            .unwrap();
        let le = b
            .build_int_compare(IntPredicate::SLE, v, self.i64t().const_int(hi as u64, true), "le")
            .unwrap();
        b.build_and(ge, le, "rng").unwrap()
    }

    /// `v == k` as an i1.
    fn eq_const(&self, v: IntValue<'ctx>, k: i64) -> IntValue<'ctx> {
        self.builder
            .build_int_compare(IntPredicate::EQ, v, self.i64t().const_int(k as u64, true), "eqk")
            .unwrap()
    }

    /// ASCII alphabetic: `A-Z` or `a-z`.
    fn is_alpha(&self, v: IntValue<'ctx>) -> IntValue<'ctx> {
        let upper = self.in_range(v, 65, 90);
        let lower = self.in_range(v, 97, 122);
        self.builder.build_or(upper, lower, "alpha").unwrap()
    }

    /// Zero-extend an i1 to the i64 Bool representation (0/1).
    fn bool_to_i64(&self, bit: IntValue<'ctx>) -> IntValue<'ctx> {
        self.builder.build_int_z_extend(bit, self.i64t(), "b64").unwrap()
    }

    /// `_array_empty()` → `{0, buf}` with a capacity-headed (cap 0) buffer.
    fn lower_array_empty(&self, elem: ZType) -> (BasicValueEnum<'ctx>, ZType) {
        let buf = self.alloc_array_buf(self.i64t().const_zero(), self.elem_bytes(&elem));
        let arr = self.make_len_ptr(self.i64t().const_zero(), buf);
        (arr.into(), ZType::Array(Box::new(elem)))
    }

    /// `_array_push(arr, x)` → a fresh `{len+1, buf}` (cap len+1) with the old
    /// elements copied and `x` stored at the end. The original buffer is untouched
    /// (matches the interpreter's copy-on-write `push`). This is the FUNCTIONAL
    /// path; the common `xs = push(xs, x)` self-assignment is lowered in-place by
    /// [`FnLower::lower_inplace_push`] (amortized O(1)).
    fn lower_array_push(
        &mut self,
        arr_expr: &MirExpr,
        value_expr: &MirExpr,
        elem: ZType,
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let (arr, _) = self.lower_expr(arr_expr)?;
        let (len, data) = self.len_ptr_parts(arr.into_struct_value());
        let (x, _) = self.lower_expr(value_expr)?;
        let elem_llvm = self.types.llvm(&elem);
        let elem_sz = self.elem_bytes(&elem);
        let b = self.builder;
        let new_len = b.build_int_add(len, self.i64t().const_int(1, false), "nlen").unwrap();
        let buf = self.alloc_array_buf(new_len, elem_sz);
        let old_bytes = b.build_int_mul(len, elem_sz, "obytes").unwrap();
        self.memcpy_bytes(buf, data, old_bytes);
        let end = unsafe { b.build_in_bounds_gep(elem_llvm, buf, &[len], "endp").unwrap() };
        b.build_store(end, x).unwrap();
        Ok((self.make_len_ptr(new_len, buf).into(), ZType::Array(Box::new(elem))))
    }

    /// In-place `name = push(name, value)`: append into `name`'s buffer, growing
    /// (capacity-doubling) only when full — amortized O(1). Sound because value
    /// semantics make `name`'s buffer uniquely owned (no aliasing observer).
    fn lower_inplace_push(
        &mut self,
        name: &str,
        value_arg: &MirExpr,
        elem: ZType,
    ) -> Result<(), String> {
        let slot = self
            .locals
            .get(name)
            .ok_or_else(|| format!("in-place push on unknown local `{name}`"))?
            .0;
        let elem_llvm = self.types.llvm(&elem);
        let elem_sz = self.elem_bytes(&elem);

        let arr = self
            .builder
            .build_load(array_struct_type(self.context), slot, "arr")
            .unwrap()
            .into_struct_value();
        let (len, ptr) = self.len_ptr_parts(arr);
        let cap = self.array_cap(ptr);
        let (v, _) = self.lower_expr(value_arg)?;

        // len < cap → append in place; else grow to max(1, cap*2) and copy.
        let cur = self.builder.get_insert_block().unwrap();
        let grow = self.context.append_basic_block(self.llvm_fn, "push.grow");
        let cont = self.context.append_basic_block(self.llvm_fn, "push.cont");
        let has_room = self
            .builder
            .build_int_compare(IntPredicate::SLT, len, cap, "room")
            .unwrap();
        self.builder.build_conditional_branch(has_room, cont, grow).unwrap();

        self.builder.position_at_end(grow);
        let cap2 = self.builder.build_int_mul(cap, self.i64t().const_int(2, false), "cap2").unwrap();
        let is_zero = self
            .builder
            .build_int_compare(IntPredicate::EQ, cap, self.i64t().const_zero(), "capz")
            .unwrap();
        let newcap = self
            .builder
            .build_select(is_zero, self.i64t().const_int(1, false), cap2, "newcap")
            .unwrap()
            .into_int_value();
        let newbuf = self.alloc_array_buf(newcap, elem_sz);
        let old_bytes = self.builder.build_int_mul(len, elem_sz, "ob").unwrap();
        self.memcpy_bytes(newbuf, ptr, old_bytes);
        self.builder.build_unconditional_branch(cont).unwrap();

        self.builder.position_at_end(cont);
        let phi = self
            .builder
            .build_phi(self.context.ptr_type(inkwell::AddressSpace::default()), "newptr")
            .unwrap();
        phi.add_incoming(&[(&ptr, cur), (&newbuf, grow)]);
        let newptr = phi.as_basic_value().into_pointer_value();

        let endp = unsafe { self.builder.build_in_bounds_gep(elem_llvm, newptr, &[len], "endp").unwrap() };
        self.builder.build_store(endp, v).unwrap();
        let newlen = self.builder.build_int_add(len, self.i64t().const_int(1, false), "nlen").unwrap();
        let newarr = self.make_len_ptr(newlen, newptr);
        self.builder.build_store(slot, newarr).unwrap();
        Ok(())
    }

    /// Extract `(len, data)` from a `{i64 len, ptr data}` value — the shared layout
    /// of both strings and arrays.
    fn len_ptr_parts(
        &self,
        s: inkwell::values::StructValue<'ctx>,
    ) -> (IntValue<'ctx>, PointerValue<'ctx>) {
        let b = self.builder;
        let len = b.build_extract_value(s, 0, "slen").unwrap().into_int_value();
        let data = b.build_extract_value(s, 1, "sdata").unwrap().into_pointer_value();
        (len, data)
    }

    /// `malloc(n)` returning the i8 buffer pointer.
    fn malloc_bytes(&self, n: IntValue<'ctx>) -> PointerValue<'ctx> {
        self.builder
            .build_call(self.malloc, &[n.into()], "buf")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value()
    }

    /// `memcpy(dst, src, n)`.
    fn memcpy_bytes(&self, dst: PointerValue<'ctx>, src: PointerValue<'ctx>, n: IntValue<'ctx>) {
        self.builder
            .build_call(self.memcpy, &[dst.into(), src.into(), n.into()], "cp")
            .unwrap();
    }

    /// Build a `{len, data}` value (used for both strings and arrays).
    fn make_len_ptr(&self, len: IntValue<'ctx>, data: PointerValue<'ctx>) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let v = b
            .build_insert_value(array_struct_type(self.context).get_undef(), len, 0, "s0")
            .unwrap();
        b.build_insert_value(v, data, 1, "s1").unwrap().into_struct_value()
    }

    /// Lower an expression that must be an `i64` (Int/Bool).
    fn lower_int(&mut self, expr: &MirExpr) -> Result<IntValue<'ctx>, String> {
        let (v, zt) = self.lower_expr(expr)?;
        if zt != ZType::Int {
            let kind = match expr {
                MirExpr::FieldAccess { field, .. } => format!("FieldAccess(.{field})"),
                MirExpr::Call { callee, .. } => format!("Call({callee})"),
                MirExpr::Index { .. } => "Index".into(),
                MirExpr::Load(n) => format!("Load({n})"),
                other => format!("{other:?}"),
            };
            return Err(format!("expected Int/Bool value, found {zt:?} from {kind}"));
        }
        Ok(v.into_int_value())
    }

    fn lower_binary(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return Ok((self.lower_logical(op, left, right)?.into(), ZType::Int));
        }
        let (lv, lt) = self.lower_expr(left)?;
        let (rv, _rt) = self.lower_expr(right)?;
        // String == / != is a byte-compare (typecheck guarantees matching types).
        if matches!(op, BinaryOp::Eq | BinaryOp::NotEq) && lt == ZType::Str {
            let eq = self.string_eq(lv.into_struct_value(), rv.into_struct_value());
            let bit = if matches!(op, BinaryOp::NotEq) {
                self.builder.build_not(eq, "sne").unwrap()
            } else {
                eq
            };
            return Ok((self.bool_to_i64(bit).into(), ZType::Int));
        }
        // Float arithmetic / comparison (f64); modulo & bitwise stay Int-only.
        if lt == ZType::Float {
            let l = lv.into_float_value();
            let r = rv.into_float_value();
            let b = self.builder;
            return Ok(match op {
                BinaryOp::Add => (b.build_float_add(l, r, "fadd").unwrap().into(), ZType::Float),
                BinaryOp::Sub => (b.build_float_sub(l, r, "fsub").unwrap().into(), ZType::Float),
                BinaryOp::Mul => (b.build_float_mul(l, r, "fmul").unwrap().into(), ZType::Float),
                BinaryOp::Div => (b.build_float_div(l, r, "fdiv").unwrap().into(), ZType::Float),
                BinaryOp::Eq => (self.fcompare(inkwell::FloatPredicate::OEQ, l, r).into(), ZType::Int),
                BinaryOp::NotEq => (self.fcompare(inkwell::FloatPredicate::ONE, l, r).into(), ZType::Int),
                BinaryOp::Lt => (self.fcompare(inkwell::FloatPredicate::OLT, l, r).into(), ZType::Int),
                BinaryOp::Lte => (self.fcompare(inkwell::FloatPredicate::OLE, l, r).into(), ZType::Int),
                BinaryOp::Gt => (self.fcompare(inkwell::FloatPredicate::OGT, l, r).into(), ZType::Int),
                BinaryOp::Gte => (self.fcompare(inkwell::FloatPredicate::OGE, l, r).into(), ZType::Int),
                _ => return Err("modulo / bitwise operators are not defined on Float".into()),
            });
        }
        if lt != ZType::Int {
            return Err(format!("binary operator not supported on {lt:?}"));
        }
        let l = lv.into_int_value();
        let r = rv.into_int_value();
        let b = self.builder;
        let result = match op {
            BinaryOp::Add => b.build_int_add(l, r, "add").unwrap(),
            BinaryOp::Sub => b.build_int_sub(l, r, "sub").unwrap(),
            BinaryOp::Mul => b.build_int_mul(l, r, "mul").unwrap(),
            BinaryOp::Div => b.build_int_signed_div(l, r, "div").unwrap(),
            BinaryOp::Mod => b.build_int_signed_rem(l, r, "mod").unwrap(),
            BinaryOp::BitAnd => b.build_and(l, r, "band").unwrap(),
            BinaryOp::BitOr => b.build_or(l, r, "bor").unwrap(),
            BinaryOp::BitXor => b.build_xor(l, r, "bxor").unwrap(),
            BinaryOp::Eq => self.compare(IntPredicate::EQ, l, r),
            BinaryOp::NotEq => self.compare(IntPredicate::NE, l, r),
            BinaryOp::Lt => self.compare(IntPredicate::SLT, l, r),
            BinaryOp::Lte => self.compare(IntPredicate::SLE, l, r),
            BinaryOp::Gt => self.compare(IntPredicate::SGT, l, r),
            BinaryOp::Gte => self.compare(IntPredicate::SGE, l, r),
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        };
        Ok((result.into(), ZType::Int))
    }

    fn compare(&self, pred: IntPredicate, l: IntValue<'ctx>, r: IntValue<'ctx>) -> IntValue<'ctx> {
        let bit = self.builder.build_int_compare(pred, l, r, "cmp").unwrap();
        self.builder.build_int_z_extend(bit, self.i64t(), "cmp64").unwrap()
    }

    fn fcompare(
        &self,
        pred: inkwell::FloatPredicate,
        l: inkwell::values::FloatValue<'ctx>,
        r: inkwell::values::FloatValue<'ctx>,
    ) -> IntValue<'ctx> {
        let bit = self.builder.build_float_compare(pred, l, r, "fcmp").unwrap();
        self.builder.build_int_z_extend(bit, self.i64t(), "fcmp64").unwrap()
    }

    /// Byte-wise string equality as an i1: same length AND `memcmp == 0`. The
    /// `memcmp` count is `min(la, lb)` so unequal lengths never over-read; the
    /// length check then makes the result correct.
    fn string_eq(
        &self,
        a: inkwell::values::StructValue<'ctx>,
        b: inkwell::values::StructValue<'ctx>,
    ) -> IntValue<'ctx> {
        let bld = self.builder;
        let (la, pa) = self.len_ptr_parts(a);
        let (lb, pb) = self.len_ptr_parts(b);
        let len_eq = bld.build_int_compare(IntPredicate::EQ, la, lb, "leneq").unwrap();
        let lt = bld.build_int_compare(IntPredicate::SLT, la, lb, "ltlen").unwrap();
        let n = bld.build_select(lt, la, lb, "minlen").unwrap().into_int_value();
        let cmp = bld
            .build_call(self.memcmp, &[pa.into(), pb.into(), n.into()], "mc")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let cmp0 = bld
            .build_int_compare(IntPredicate::EQ, cmp, self.context.i32_type().const_zero(), "cmp0")
            .unwrap();
        bld.build_and(len_eq, cmp0, "streq").unwrap()
    }

    fn lower_logical(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        let result = self.entry_alloca("logic", self.i64t().into());
        let l = self.lower_int(left)?;
        let l_bool = self
            .builder
            .build_int_compare(IntPredicate::NE, l, self.i64t().const_zero(), "lb")
            .unwrap();
        let rhs_bb = self.context.append_basic_block(self.llvm_fn, "logic.rhs");
        let short_bb = self.context.append_basic_block(self.llvm_fn, "logic.short");
        let cont_bb = self.context.append_basic_block(self.llvm_fn, "logic.cont");
        match op {
            BinaryOp::And => self
                .builder
                .build_conditional_branch(l_bool, rhs_bb, short_bb)
                .unwrap(),
            BinaryOp::Or => self
                .builder
                .build_conditional_branch(l_bool, short_bb, rhs_bb)
                .unwrap(),
            _ => unreachable!(),
        };
        self.builder.position_at_end(short_bb);
        let short_value = if matches!(op, BinaryOp::And) {
            self.i64t().const_zero()
        } else {
            self.i64t().const_int(1, false)
        };
        self.builder.build_store(result, short_value).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        self.builder.position_at_end(rhs_bb);
        let r = self.lower_int(right)?;
        let r_bool = self
            .builder
            .build_int_compare(IntPredicate::NE, r, self.i64t().const_zero(), "rb")
            .unwrap();
        let r_i64 = self.builder.build_int_z_extend(r_bool, self.i64t(), "rb64").unwrap();
        self.builder.build_store(result, r_i64).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        self.builder.position_at_end(cont_bb);
        Ok(self
            .builder
            .build_load(self.i64t(), result, "logic.val")
            .unwrap()
            .into_int_value())
    }

    fn lower_cond(&mut self, expr: &MirExpr) -> Result<IntValue<'ctx>, String> {
        let v = self.lower_int(expr)?;
        Ok(self
            .builder
            .build_int_compare(IntPredicate::NE, v, self.i64t().const_zero(), "tobool")
            .unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jit_toolchain_smoke() {
        assert_eq!(jit_smoke_constant(42), 42);
        assert_eq!(jit_smoke_constant(-7), -7);
    }
}
