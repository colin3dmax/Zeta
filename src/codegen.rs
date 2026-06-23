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
//! `int_to_string` (self-contained, no libc), and the `ascii_is_*` predicates.
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
    /// A raw pointer `*T` (unsafe, native-only). Represented as an LLVM opaque
    /// `ptr`; the pointee `T` drives the width of `ptr_read`/`ptr_write` loads
    /// and the element stride of `ptr_offset`. Not owned — never cloned/dropped.
    Ptr(Box<ZType>),
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
    /// Mangled instance name (`HashMap$Str_Int`) → its concrete type arguments.
    /// Recorded at monomorphization so `unify_ztype` can recover a generic
    /// struct/enum parameter's type args from a concrete instance — letting a
    /// type parameter be inferred from a struct-typed argument (e.g. `V` from a
    /// `HashMap<K, V>` value) and not only from a value of that type directly.
    instances: RefCell<HashMap<String, Vec<ZType>>>,
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
            instances: RefCell::new(HashMap::new()),
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
            ZType::Ptr(_) => self.context.ptr_type(inkwell::AddressSpace::default()).into(),
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
        if let Some(pointee) = crate::type_syntax::ptr_parts(ann) {
            return Ok(ZType::Ptr(Box::new(self.resolve_ann_ztype(pointee)?)));
        }
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
            if base == "Array" && arg_strs.len() == 1 {
                return Ok(ZType::Array(Box::new(self.resolve_ann_ztype(arg_strs[0])?)));
            }
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
            if base == "Array" && arg_strs.len() == 1 {
                return Ok(ZType::Array(Box::new(
                    self.resolve_template_type(arg_strs[0], subst)?,
                )));
            }
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
        self.instances
            .borrow_mut()
            .entry(mangled.clone())
            .or_insert_with(|| args.to_vec());
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
        self.instances
            .borrow_mut()
            .entry(mangled.clone())
            .or_insert_with(|| args.to_vec());
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
    if let Some(pointee) = crate::type_syntax::ptr_parts(text) {
        return Ok(ZType::Ptr(Box::new(parse_ztype(pointee, struct_names, enum_names)?)));
    }
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

/// The `{ ptr fn, ptr env, ptr drop_thunk, ptr clone_thunk }` value type used for
/// all closures. `fn`/`env` are the call ABI (indices 0/1, unchanged); the two
/// thunks carry the per-lambda capture layout so a type-level closure drop/clone
/// can deep-free / deep-copy the heap environment without knowing the captures.
/// Both thunks are null for a zero/undef closure (then drop/clone are no-ops).
fn closure_struct_type(context: &Context) -> StructType {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    context.struct_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false)
}

/// Sentinel refcount stored in a string literal's global header: clone/drop skip
/// any string whose refcount equals this, so the static global is never bumped or
/// freed. Heap strings start at rc = 1 and can never reach this value.
const STATIC_STR_RC: u64 = 0x8000_0000_0000_0000; // i64::MIN

/// Unify a generic parameter type STRING against a concrete `ZType`, binding any
/// type parameters it names into `subst` (recursing through tuple/function types).
fn unify_ztype(
    generic_str: &str,
    concrete: &ZType,
    type_params: &[String],
    instances: &HashMap<String, Vec<ZType>>,
    subst: &mut HashMap<String, ZType>,
) {
    if type_params.iter().any(|p| p == generic_str) {
        subst.insert(generic_str.to_string(), concrete.clone());
        return;
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(generic_str) {
        if let ZType::Tuple(elems) = concrete {
            for (p, c) in parts.iter().zip(elems) {
                unify_ztype(p, c, type_params, instances, subst);
            }
        }
        return;
    }
    if let Some((params, ret)) = crate::type_syntax::fn_parts(generic_str) {
        if let ZType::Closure(cparams, cret) = concrete {
            for (p, c) in params.iter().zip(cparams) {
                unify_ztype(p, c, type_params, instances, subst);
            }
            unify_ztype(ret, cret, type_params, instances, subst);
        }
        return;
    }
    if let Some((base, args)) = crate::type_syntax::generic_parts(generic_str) {
        // `Array<E>` vs a concrete array: recurse to bind a type-parameter element.
        if base == "Array" && args.len() == 1 {
            if let ZType::Array(elem) = concrete {
                unify_ztype(args[0], elem, type_params, instances, subst);
            }
            return;
        }
        // A generic struct/enum parameter (`HashMap<K, V>`) vs a concrete
        // monomorphized instance: recover the instance's recorded type args and
        // recurse, so a parameter appearing only inside an aggregate type still
        // binds (e.g. infer `V` from a `HashMap<K, V>`-typed argument).
        if let ZType::Struct(name) | ZType::Enum(name) = concrete {
            if let Some(concrete_args) = instances.get(name) {
                for (p, c) in args.iter().zip(concrete_args) {
                    unify_ztype(p, c, type_params, instances, subst);
                }
            }
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
        ZType::Ptr(elem) => format!("Ptr{}", zty_mangle(elem)),
    }
}

/// The base type name used to dispatch a trait method on a value of ZType `zt`.
/// Matches the interpreter's `value_type_base` and the `base_name` of an impl's
/// target so native and the oracle agree. (Bool folds into `Int` here, so trait
/// impls on `Bool` receivers are not dispatched natively — a rare case.)
fn zty_base_name(zt: &ZType) -> String {
    match zt {
        ZType::Int => "Int".to_string(),
        ZType::Float => "Float".to_string(),
        ZType::Str => "String".to_string(),
        ZType::Struct(name) => name.clone(),
        ZType::Enum(name) => name.clone(),
        ZType::Array(_) => "Array".to_string(),
        ZType::Tuple(_) => "Tuple".to_string(),
        ZType::Closure(..) => "Fn".to_string(),
        ZType::Ptr(_) => "Ptr".to_string(),
    }
}

/// Access width in bits for an `mmio_*_{byte,word,dword}` builtin.
fn mmio_bits(callee: &str) -> u32 {
    if callee.ends_with("word") {
        if callee.ends_with("dword") {
            64
        } else {
            32
        }
    } else {
        8
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
        ZType::Ptr(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
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
    // Generic functions can't be lowered as-is (LLVM is statically typed); they
    // are kept aside and monomorphized on demand at each call site.
    let generics: HashMap<String, &MirFunction> = program
        .functions
        .iter()
        .filter(|f| !f.type_params.is_empty())
        .map(|f| (f.name.clone(), f))
        .collect();
    let specialized: RefCell<HashMap<String, FunctionValue>> = RefCell::new(HashMap::new());
    // Trait method names — a call to one dispatches by the receiver's static
    // ZType base to the flattened impl `{method}${TypeBase}` (a concrete fn).
    let trait_methods: HashSet<String> = program.trait_methods.iter().cloned().collect();

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

    // Pass 2: lower each concrete body. Extern functions have no body — pass 1's
    // declaration stands and the linker resolves the symbol.
    for function in &program.functions {
        if !function.type_params.is_empty() || function.is_extern {
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
            trait_methods: &trait_methods,
            specialized: &specialized,
            malloc,
            free,
            memcpy,
            memcmp,
            llvm_fn,
            entry_bb,
            lambda_count: 0,
            locals: HashMap::new(),
            move_plan: crate::move_analysis::analyze(function),
            moved_flags: HashMap::new(),
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
            // A param that gets moved away later needs its flag zeroed up front.
            lower.bind_moved_flag(&param.name);
        }

        let terminated = lower
            .lower_stmts(&function.body)
            .map_err(|e| format!("in `{}`: {e}", function.name))?;
        if !terminated {
            let ret = &types.returns[&function.name];
            // Implicit fall-through return: the value is a zero/zeroinitializer
            // (aliases no local), so drop every managed local.
            lower.free_live_managed_except(None);
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
    /// Trait method names — a call to one dispatches by the first argument's
    /// ZType base to a flattened impl function `{method}${TypeBase}`.
    trait_methods: &'a HashSet<String>,
    /// Cache of monomorphized instances: mangled name → lifted LLVM function.
    /// Shared (RefCell) across all `FnLower`s so each (generic, type args) pair
    /// is generated once and reused.
    specialized: &'a RefCell<HashMap<String, FunctionValue<'ctx>>>,
    malloc: FunctionValue<'ctx>,
    free: FunctionValue<'ctx>,
    memcpy: FunctionValue<'ctx>,
    memcmp: FunctionValue<'ctx>,
    llvm_fn: FunctionValue<'ctx>,
    entry_bb: BasicBlock<'ctx>,
    /// Monotonic counter for naming lambdas lifted out of this function.
    lambda_count: u32,
    /// local name → (alloca slot, type)
    locals: HashMap<String, (PointerValue<'ctx>, ZType)>,
    /// Move-on-last-use plan for this function body: which `Load` nodes may MOVE
    /// (skip the clone) and which locals therefore carry a runtime moved-flag.
    move_plan: crate::move_analysis::MovePlan,
    /// Moved-flag slot (i1) per flagged local: 0 = still owned (drop it), 1 =
    /// already moved out (skip its scope-exit drop). Reset to 0 each time the
    /// local is (re)bound; set to 1 at the move site.
    moved_flags: HashMap<String, PointerValue<'ctx>>,
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
            ZType::Ptr(_) => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null()
                .into(),
        }
    }

    /// Apply value-semantics at a binding point: if `value` is an array, return a
    /// deep copy (fresh malloc'd buffer) so the new owner is independent; other
    /// types are already value types in LLVM and pass through. (String elements
    /// are themselves immutable, so copying their `{len,ptr}` is safe sharing.)
    fn bind_value(&self, value: BasicValueEnum<'ctx>, zt: &ZType) -> BasicValueEnum<'ctx> {
        self.clone_value(value, zt)
    }

    /// Whether `expr`'s value is already a uniquely-owned heap tree the binding
    /// can TAKE without cloning. A container literal (array/tuple/struct/enum) is
    /// self-owned because its managed members are cloned at construction; a call
    /// result MOVES ownership from the callee. Everything else (a `Load`, a
    /// field/index read, a string literal) aliases memory owned elsewhere and
    /// must be cloned.
    fn is_owned_source(&self, expr: &MirExpr, zt: &ZType) -> bool {
        match expr {
            MirExpr::ArrayLiteral { .. }
            | MirExpr::Tuple { .. }
            | MirExpr::StructLiteral { .. }
            | MirExpr::EnumVariant { .. } => true,
            MirExpr::Call { .. } => self.needs_drop(zt),
            _ => false,
        }
    }

    /// Like [`bind_value`] but skips the clone when `expr` already produced a
    /// uniquely-owned value (see [`is_owned_source`]).
    fn bind_owned(
        &self,
        expr: &MirExpr,
        value: BasicValueEnum<'ctx>,
        zt: &ZType,
    ) -> BasicValueEnum<'ctx> {
        if self.is_owned_source(expr, zt) {
            return value;
        }
        // Move-on-last-use: a managed local read for the last time in an
        // ownership-transferring position is MOVED, not cloned — take its value
        // and flag it moved so its scope-exit drop is suppressed. Soundness is
        // carried entirely by `move_analysis` (the read is dead-after on every
        // path) plus the runtime flag (the drop is skipped iff the move ran).
        if self.needs_drop(zt) {
            if let MirExpr::Load(name) = expr {
                if self.move_plan.is_move(expr) {
                    self.mark_moved(name);
                    return value;
                }
            }
        }
        self.bind_value(value, zt)
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
        // 16-byte header: `[ i64 cap | i64 refcount ]`. Refcount starts at 1 so the
        // buffer is uniquely owned; clone bumps it (O(1) share) and drop decrements,
        // freeing only at zero. In-place mutation copies-on-write when shared.
        let header = self.i64t().const_int(16, false);
        let elem_bytes = b.build_int_mul(cap, elem_size, "capbytes").unwrap();
        let total = b.build_int_add(header, elem_bytes, "totbytes").unwrap();
        let base = self.malloc_bytes(total);
        b.build_store(base, cap).unwrap();
        let rc_ptr = unsafe {
            b.build_in_bounds_gep(self.context.i8_type(), base, &[self.i64t().const_int(8, false)], "rcslot")
                .unwrap()
        };
        b.build_store(rc_ptr, self.i64t().const_int(1, false)).unwrap();
        unsafe {
            b.build_in_bounds_gep(self.context.i8_type(), base, &[header], "elems")
                .unwrap()
        }
    }

    /// Read the capacity header stored 16 bytes before the elements pointer.
    fn array_cap(&self, elems: PointerValue<'ctx>) -> IntValue<'ctx> {
        let b = self.builder;
        let back = self.i64t().const_int((-16i64) as u64, true);
        let hdr = unsafe {
            b.build_in_bounds_gep(self.context.i8_type(), elems, &[back], "caphdr")
                .unwrap()
        };
        b.build_load(self.i64t(), hdr, "cap").unwrap().into_int_value()
    }

    /// Pointer to the refcount field (8 bytes before the elements pointer).
    fn array_rc_ptr(&self, elems: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let back = self.i64t().const_int((-8i64) as u64, true);
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), elems, &[back], "rchdr")
                .unwrap()
        }
    }

    /// Increment an array buffer's refcount (share it). Returns nothing.
    fn array_rc_inc(&self, elems: PointerValue<'ctx>) {
        let rc_ptr = self.array_rc_ptr(elems);
        let rc = self.builder.build_load(self.i64t(), rc_ptr, "rc").unwrap().into_int_value();
        let inc = self.builder.build_int_add(rc, self.i64t().const_int(1, false), "rci").unwrap();
        self.builder.build_store(rc_ptr, inc).unwrap();
    }

    /// Decrement an array buffer's refcount; return the i1 `rc == 0` (caller frees).
    fn array_rc_dec_is_zero(&self, elems: PointerValue<'ctx>) -> IntValue<'ctx> {
        let rc_ptr = self.array_rc_ptr(elems);
        let rc = self.builder.build_load(self.i64t(), rc_ptr, "rc").unwrap().into_int_value();
        let dec = self.builder.build_int_sub(rc, self.i64t().const_int(1, false), "rcd").unwrap();
        self.builder.build_store(rc_ptr, dec).unwrap();
        self.builder.build_int_compare(IntPredicate::EQ, dec, self.i64t().const_zero(), "rcz").unwrap()
    }

    /// Allocate a heap STRING buffer of `n` bytes with an 8-byte refcount header
    /// (rc = 1). Returns the bytes pointer (header is one i64 before it). Strings
    /// are immutable, so sharing is pure refcount with no copy-on-write.
    fn alloc_str_buf(&self, n: IntValue<'ctx>) -> PointerValue<'ctx> {
        let b = self.builder;
        let total = b.build_int_add(self.i64t().const_int(8, false), n, "strtot").unwrap();
        let base = self.malloc_bytes(total);
        b.build_store(base, self.i64t().const_int(1, false)).unwrap();
        unsafe {
            b.build_in_bounds_gep(self.context.i8_type(), base, &[self.i64t().const_int(8, false)], "strbytes")
                .unwrap()
        }
    }

    /// Free a heap string buffer (recover the base 8 bytes before the data ptr).
    fn free_str_data(&self, data: PointerValue<'ctx>) {
        let back = self.i64t().const_int((-8i64) as u64, true);
        let base = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[back], "strbase")
                .unwrap()
        };
        self.builder.build_call(self.free, &[base.into()], "").unwrap();
    }

    /// Pointer to a string buffer's refcount field (8 bytes before the data ptr).
    fn str_rc_ptr(&self, data: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let back = self.i64t().const_int((-8i64) as u64, true);
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[back], "strrc")
                .unwrap()
        }
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

    /// Allocate (once) and ZERO a flagged local's moved-flag at the current
    /// point — called whenever the local is (re)bound, so it starts owned. A
    /// no-op for locals the move plan never moves.
    fn bind_moved_flag(&mut self, name: &str) {
        if !self.move_plan.is_flagged(name) {
            return;
        }
        let slot = match self.moved_flags.get(name) {
            Some(&slot) => slot,
            None => {
                let slot =
                    self.entry_alloca(&format!("{name}.moved"), self.context.bool_type().into());
                self.moved_flags.insert(name.to_string(), slot);
                slot
            }
        };
        self.builder
            .build_store(slot, self.context.bool_type().const_zero())
            .unwrap();
    }

    /// Record that a flagged local's value has been MOVED out (set its flag to
    /// 1), so its scope-exit drop is skipped. A no-op for unflagged locals.
    fn mark_moved(&self, name: &str) {
        if let Some(&slot) = self.moved_flags.get(name) {
            self.builder
                .build_store(slot, self.context.bool_type().const_int(1, false))
                .unwrap();
        }
    }

    /// Drop a managed local unless its moved-flag says it was already moved out.
    /// Plain `drop_local` when the local carries no flag.
    fn drop_local_guarded(&self, name: &str, slot: PointerValue<'ctx>, zt: &ZType) {
        let Some(&flag) = self.moved_flags.get(name) else {
            self.drop_local(slot, zt);
            return;
        };
        let moved = self
            .builder
            .build_load(self.context.bool_type(), flag, "moved")
            .unwrap()
            .into_int_value();
        let live = self
            .builder
            .build_int_compare(IntPredicate::EQ, moved, self.context.bool_type().const_zero(), "notmoved")
            .unwrap();
        let drop_bb = self.context.append_basic_block(self.llvm_fn, "drop.live");
        let cont_bb = self.context.append_basic_block(self.llvm_fn, "drop.cont");
        self.builder.build_conditional_branch(live, drop_bb, cont_bb).unwrap();
        self.builder.position_at_end(drop_bb);
        self.drop_local(slot, zt);
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        self.builder.position_at_end(cont_bb);
    }

    fn lower_stmts(&mut self, stmts: &[MirStmt]) -> Result<bool, String> {
        for stmt in stmts {
            if self.lower_stmt(stmt)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ===== Value-semantics memory management: generated per-type drop/clone =====
    //
    // The Rust-style model realized through value semantics: each value uniquely
    // owns its heap, freed deterministically at scope exit (no GC). For every
    // MANAGED type we generate ONE recursive `@__drop_T` / `@__clone_T` module
    // function (cached, built once). They recurse on DATA via calls — so a
    // recursive type (struct reachable from itself through an array field) drops
    // to its actual finite depth at RUNTIME, with no codegen blow-up. Enum and
    // Closure heap stay a conservative leak for now (their payload/env layout
    // isn't statically known at every site).

    /// Whether `zt` owns heap that drop/clone must manage. Terminates: `Array`
    /// short-circuits (no recursion into the element), and inline struct/tuple
    /// nesting is acyclic (it would otherwise be infinite size).
    fn needs_drop(&self, zt: &ZType) -> bool {
        match zt {
            ZType::Str | ZType::Array(_) => true,
            ZType::Tuple(elems) => elems.iter().any(|e| self.needs_drop(e)),
            ZType::Struct(name) => {
                let fields = self.types.structs.borrow()[name].fields.clone();
                fields.iter().any(|(_, t)| self.needs_drop(t))
            }
            // An enum needs drop if any variant carries managed heap: a Str/Array
            // payload (its buffer), or a Struct payload (always heap-BOXED via p1,
            // so the box must be freed even when the struct itself has no managed
            // fields). Int/Bool/Float payloads and payload-less variants are inline.
            ZType::Enum(name) => {
                let variants = self.types.enums.borrow()[name].clone();
                variants.iter().any(|(_, p)| match p {
                    Some(pt) => self.needs_drop(pt) || matches!(pt, ZType::Struct(_)),
                    None => false,
                })
            }
            // A closure always owns a heap environment (malloc'd at the lambda site),
            // so it always needs drop — its drop-thunk frees the env (+ captures).
            ZType::Closure(_, _) => true,
            _ => false,
        }
    }

    /// Emit `free` for an array elements pointer: recover the malloc base
    /// (`data - 16` cap+refcount header, see [`alloc_array_buf`]) and free it.
    fn free_array_data(&self, data: PointerValue<'ctx>) {
        let back = self.i64t().const_int((-16i64) as u64, true);
        let base = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[back], "freebase")
                .unwrap()
        };
        self.builder.build_call(self.free, &[base.into()], "").unwrap();
    }

    /// Copy-on-write: make the array in `slot` uniquely owned before an in-place
    /// mutation. If its refcount is >1 (a clone shares the buffer), deep-copy the
    /// buffer (preserving capacity), drop one reference from the shared original,
    /// and store the unique copy back into `slot`. Returns the now-unique
    /// `(len, data)`. Managed-element arrays are always rc==1 (clone deep-copies),
    /// so they never hit the copy path; the scalar memcpy here is therefore sound.
    fn cow_make_unique(&self, slot: PointerValue<'ctx>, elem: &ZType) -> (IntValue<'ctx>, PointerValue<'ctx>) {
        let arr = self
            .builder
            .build_load(array_struct_type(self.context), slot, "cowarr")
            .unwrap()
            .into_struct_value();
        let (len, data) = self.len_ptr_parts(arr);
        let rc_ptr = self.array_rc_ptr(data);
        let rc = self.builder.build_load(self.i64t(), rc_ptr, "cowrc").unwrap().into_int_value();
        let shared = self
            .builder
            .build_int_compare(IntPredicate::SGT, rc, self.i64t().const_int(1, false), "cowsh")
            .unwrap();
        let entry = self.builder.get_insert_block().unwrap();
        let copy_blk = self.context.append_basic_block(self.llvm_fn, "cow.copy");
        let done = self.context.append_basic_block(self.llvm_fn, "cow.done");
        self.builder.build_conditional_branch(shared, copy_blk, done).unwrap();

        self.builder.position_at_end(copy_blk);
        let cap = self.array_cap(data);
        let elem_size = self.elem_bytes(elem);
        let newbuf = self.alloc_array_buf(cap, elem_size);
        let bytes = self.builder.build_int_mul(len, elem_size, "cowbytes").unwrap();
        self.memcpy_bytes(newbuf, data, bytes);
        // rc > 1 here, so after this decrement the original still has an owner.
        let dec = self.builder.build_int_sub(rc, self.i64t().const_int(1, false), "cowdec").unwrap();
        self.builder.build_store(rc_ptr, dec).unwrap();
        let newval = self.make_len_ptr(len, newbuf);
        self.builder.build_store(slot, newval).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();
        let copy_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(done);
        let phi = self
            .builder
            .build_phi(self.context.ptr_type(inkwell::AddressSpace::default()), "cowptr")
            .unwrap();
        phi.add_incoming(&[(&data, entry), (&newbuf, copy_end)]);
        (len, phi.as_basic_value().into_pointer_value())
    }

    /// Get (or generate once) the `void @__drop_T(T)` destructor for a managed
    /// type. Inserted into the cache BEFORE its body is emitted so a recursive
    /// type's drop can call itself.
    fn get_or_build_drop(&self, zt: &ZType) -> Option<FunctionValue<'ctx>> {
        if !self.needs_drop(zt) {
            return None;
        }
        let mangled = format!("__drop_{}", zty_mangle(zt));
        if let Some(f) = self.specialized.borrow().get(&mangled) {
            return Some(*f);
        }
        let param_ty = self.types.llvm(zt);
        let fn_ty = self.context.void_type().fn_type(&[param_ty.into()], false);
        let func = self.module.add_function(&mangled, fn_ty, None);
        self.specialized.borrow_mut().insert(mangled, func);
        let saved = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        let v = func.get_nth_param(0).expect("drop param");
        self.emit_drop_body(v, zt, func);
        self.builder.build_return(None).unwrap();
        if let Some(block) = saved {
            self.builder.position_at_end(block);
        }
        Some(func)
    }

    fn emit_drop_body(&self, v: BasicValueEnum<'ctx>, zt: &ZType, func: FunctionValue<'ctx>) {
        match zt {
            ZType::Str => {
                // Refcount drop: decrement and free only at zero. Static literals
                // (sentinel rc) are skipped entirely — never decremented or freed.
                let data = self
                    .builder
                    .build_extract_value(v.into_struct_value(), 1, "sd")
                    .unwrap()
                    .into_pointer_value();
                let rc_ptr = self.str_rc_ptr(data);
                let rc = self.builder.build_load(self.i64t(), rc_ptr, "sdrc").unwrap().into_int_value();
                let is_static = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, rc, self.i64t().const_int(STATIC_STR_RC, false), "sdstat")
                    .unwrap();
                let live = self.context.append_basic_block(func, "sd.live");
                let free_blk = self.context.append_basic_block(func, "sd.free");
                let done = self.context.append_basic_block(func, "sd.done");
                self.builder.build_conditional_branch(is_static, done, live).unwrap();
                self.builder.position_at_end(live);
                let dec = self.builder.build_int_sub(rc, self.i64t().const_int(1, false), "sddec").unwrap();
                self.builder.build_store(rc_ptr, dec).unwrap();
                let is_zero = self.builder.build_int_compare(IntPredicate::EQ, dec, self.i64t().const_zero(), "sdz").unwrap();
                self.builder.build_conditional_branch(is_zero, free_blk, done).unwrap();
                self.builder.position_at_end(free_blk);
                self.free_str_data(data);
                self.builder.build_unconditional_branch(done).unwrap();
                self.builder.position_at_end(done);
            }
            ZType::Array(elem) => {
                let sv = v.into_struct_value();
                let len = self.builder.build_extract_value(sv, 0, "al").unwrap().into_int_value();
                let data = self.builder.build_extract_value(sv, 1, "ad").unwrap().into_pointer_value();
                // Refcount drop: decrement; only the last owner (rc → 0) drops the
                // elements and frees the buffer. Shared buffers (managed elements are
                // always rc==1 since clone deep-copies them) just lose a reference.
                let is_zero = self.array_rc_dec_is_zero(data);
                let free_blk = self.context.append_basic_block(func, "ad.free");
                let done = self.context.append_basic_block(func, "ad.done");
                self.builder.build_conditional_branch(is_zero, free_blk, done).unwrap();
                self.builder.position_at_end(free_blk);
                if let Some(df) = self.get_or_build_drop(elem) {
                    // for i in 0..len: @__drop_elem(load data[i])
                    let elem_llvm = self.types.llvm(elem);
                    let entry = self.builder.get_insert_block().unwrap();
                    let head = self.context.append_basic_block(func, "d.head");
                    let body = self.context.append_basic_block(func, "d.body");
                    let exit = self.context.append_basic_block(func, "d.exit");
                    self.builder.build_unconditional_branch(head).unwrap();
                    self.builder.position_at_end(head);
                    let phi = self.builder.build_phi(self.i64t(), "i").unwrap();
                    phi.add_incoming(&[(&self.i64t().const_zero(), entry)]);
                    let i = phi.as_basic_value().into_int_value();
                    let c = self.builder.build_int_compare(IntPredicate::SLT, i, len, "c").unwrap();
                    self.builder.build_conditional_branch(c, body, exit).unwrap();
                    self.builder.position_at_end(body);
                    let ep = unsafe { self.builder.build_in_bounds_gep(elem_llvm, data, &[i], "ep").unwrap() };
                    let ev = self.builder.build_load(elem_llvm, ep, "ev").unwrap();
                    self.builder.build_call(df, &[ev.into()], "").unwrap();
                    let nx = self.builder.build_int_add(i, self.i64t().const_int(1, false), "nx").unwrap();
                    phi.add_incoming(&[(&nx, self.builder.get_insert_block().unwrap())]);
                    self.builder.build_unconditional_branch(head).unwrap();
                    self.builder.position_at_end(exit);
                }
                self.free_array_data(data);
                self.builder.build_unconditional_branch(done).unwrap();
                self.builder.position_at_end(done);
            }
            ZType::Tuple(elems) => {
                let sv = v.into_struct_value();
                for (i, e) in elems.iter().enumerate() {
                    if let Some(df) = self.get_or_build_drop(e) {
                        let fv = self.builder.build_extract_value(sv, i as u32, "te").unwrap();
                        self.builder.build_call(df, &[fv.into()], "").unwrap();
                    }
                }
            }
            ZType::Struct(name) => {
                let fields = self.types.structs.borrow()[name].fields.clone();
                let sv = v.into_struct_value();
                for (i, (_, ft)) in fields.iter().enumerate() {
                    if let Some(df) = self.get_or_build_drop(ft) {
                        let fv = self.builder.build_extract_value(sv, i as u32, "fe").unwrap();
                        self.builder.build_call(df, &[fv.into()], "").unwrap();
                    }
                }
            }
            // `{tag, p0, p1}`: switch on the tag and drop the active variant's
            // payload. Str/Array → reconstruct {len, ptr} from (p0, p1) and call its
            // drop (frees the buffer). Struct → load the boxed struct from p1, drop
            // its managed fields, then free the box itself (a plain malloc).
            ZType::Enum(name) => {
                let variants = self.types.enums.borrow()[name].clone();
                let sv = v.into_struct_value();
                let tag = self.builder.build_extract_value(sv, 0, "etag").unwrap().into_int_value();
                let p0 = self.builder.build_extract_value(sv, 1, "ep0").unwrap().into_int_value();
                let p1 = self.builder.build_extract_value(sv, 2, "ep1").unwrap().into_pointer_value();
                let end = self.context.append_basic_block(func, "e.end");
                let default = self.context.append_basic_block(func, "e.def");
                let mut cases = Vec::new();
                let mut managed = Vec::new();
                for (i, (_, payload)) in variants.iter().enumerate() {
                    let pt = match payload {
                        Some(pt) if self.needs_drop(pt) || matches!(pt, ZType::Struct(_)) => pt.clone(),
                        _ => continue,
                    };
                    let blk = self.context.append_basic_block(func, "e.case");
                    cases.push((self.i64t().const_int(i as u64, false), blk));
                    managed.push((blk, pt));
                }
                self.builder.build_switch(tag, default, &cases).unwrap();
                for (blk, pt) in managed {
                    self.builder.position_at_end(blk);
                    match &pt {
                        ZType::Str | ZType::Array(_) => {
                            let payload = self.make_len_ptr(p0, p1);
                            if let Some(df) = self.get_or_build_drop(&pt) {
                                self.builder.build_call(df, &[payload.into()], "").unwrap();
                            }
                        }
                        ZType::Struct(sname) => {
                            if let Some(df) = self.get_or_build_drop(&pt) {
                                let struct_ty = self.types.struct_llvm(sname);
                                let loaded = self.builder.build_load(struct_ty, p1, "boxload").unwrap();
                                self.builder.build_call(df, &[loaded.into()], "").unwrap();
                            }
                            self.builder.build_call(self.free, &[p1.into()], "").unwrap();
                        }
                        _ => {}
                    }
                    self.builder.build_unconditional_branch(end).unwrap();
                }
                self.builder.position_at_end(default);
                self.builder.build_unconditional_branch(end).unwrap();
                self.builder.position_at_end(end);
            }
            // `{fn, env, drop_thunk, clone_thunk}`: delegate to the per-lambda drop
            // thunk (which drops captures + frees env). Guard the null thunk of a
            // zero/undef closure.
            ZType::Closure(_, _) => {
                let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                let sv = v.into_struct_value();
                let env = self.builder.build_extract_value(sv, 1, "cenv").unwrap().into_pointer_value();
                let thunk = self.builder.build_extract_value(sv, 2, "cdrop").unwrap().into_pointer_value();
                let is_null = self.builder.build_is_null(thunk, "tn").unwrap();
                let do_blk = self.context.append_basic_block(func, "cd.do");
                let end = self.context.append_basic_block(func, "cd.end");
                self.builder.build_conditional_branch(is_null, end, do_blk).unwrap();
                self.builder.position_at_end(do_blk);
                let fn_ty = self.context.void_type().fn_type(&[ptr_ty.into()], false);
                self.builder.build_indirect_call(fn_ty, thunk, &[env.into()], "").unwrap();
                self.builder.build_unconditional_branch(end).unwrap();
                self.builder.position_at_end(end);
            }
            _ => {}
        }
    }

    /// Get (or generate once) the `T @__clone_T(T)` deep-copy for a managed type.
    fn get_or_build_clone(&self, zt: &ZType) -> Option<FunctionValue<'ctx>> {
        if !self.needs_drop(zt) {
            return None;
        }
        let mangled = format!("__clone_{}", zty_mangle(zt));
        if let Some(f) = self.specialized.borrow().get(&mangled) {
            return Some(*f);
        }
        let param_ty = self.types.llvm(zt);
        let fn_ty = param_ty.fn_type(&[param_ty.into()], false);
        let func = self.module.add_function(&mangled, fn_ty, None);
        self.specialized.borrow_mut().insert(mangled, func);
        let saved = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        let v = func.get_nth_param(0).expect("clone param");
        let r = self.emit_clone_body(v, zt, func);
        self.builder.build_return(Some(&r)).unwrap();
        if let Some(block) = saved {
            self.builder.position_at_end(block);
        }
        Some(func)
    }

    fn emit_clone_body(
        &self,
        v: BasicValueEnum<'ctx>,
        zt: &ZType,
        func: FunctionValue<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        match zt {
            ZType::Str => {
                // Immutable string → share by refcount (no copy). Bump rc unless the
                // buffer is a static literal (sentinel rc, left untouched).
                let sv = v.into_struct_value();
                let data = self.builder.build_extract_value(sv, 1, "ss").unwrap().into_pointer_value();
                let rc_ptr = self.str_rc_ptr(data);
                let rc = self.builder.build_load(self.i64t(), rc_ptr, "src").unwrap().into_int_value();
                let is_static = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, rc, self.i64t().const_int(STATIC_STR_RC, false), "sstat")
                    .unwrap();
                let inc_blk = self.context.append_basic_block(func, "sc.inc");
                let cont = self.context.append_basic_block(func, "sc.cont");
                self.builder.build_conditional_branch(is_static, cont, inc_blk).unwrap();
                self.builder.position_at_end(inc_blk);
                let inc = self.builder.build_int_add(rc, self.i64t().const_int(1, false), "srci").unwrap();
                self.builder.build_store(rc_ptr, inc).unwrap();
                self.builder.build_unconditional_branch(cont).unwrap();
                self.builder.position_at_end(cont);
                v
            }
            ZType::Array(elem) => {
                let sv = v.into_struct_value();
                // Scalar elements: copy-on-write share — bump the refcount and hand
                // back the SAME buffer (O(1), no deep copy). In-place mutation later
                // makes a unique copy if the buffer is still shared.
                if !self.needs_drop(elem) {
                    let src = self.builder.build_extract_value(sv, 1, "cs").unwrap().into_pointer_value();
                    self.array_rc_inc(src);
                    return v;
                }
                // Managed elements: deep copy so each element heap is independent
                // (kept rc==1, so it never aliases — element drop/clone stay simple).
                let len = self.builder.build_extract_value(sv, 0, "cl").unwrap().into_int_value();
                let src = self.builder.build_extract_value(sv, 1, "cs").unwrap().into_pointer_value();
                let elem_size = self.elem_bytes(elem);
                let dst = self.alloc_array_buf(len, elem_size);
                if let Some(cf) = self.get_or_build_clone(elem) {
                    // for i in 0..len: dst[i] = @__clone_elem(src[i])
                    let elem_llvm = self.types.llvm(elem);
                    let entry = self.builder.get_insert_block().unwrap();
                    let head = self.context.append_basic_block(func, "c.head");
                    let body = self.context.append_basic_block(func, "c.body");
                    let exit = self.context.append_basic_block(func, "c.exit");
                    self.builder.build_unconditional_branch(head).unwrap();
                    self.builder.position_at_end(head);
                    let phi = self.builder.build_phi(self.i64t(), "i").unwrap();
                    phi.add_incoming(&[(&self.i64t().const_zero(), entry)]);
                    let i = phi.as_basic_value().into_int_value();
                    let c = self.builder.build_int_compare(IntPredicate::SLT, i, len, "c").unwrap();
                    self.builder.build_conditional_branch(c, body, exit).unwrap();
                    self.builder.position_at_end(body);
                    let sp = unsafe { self.builder.build_in_bounds_gep(elem_llvm, src, &[i], "sp").unwrap() };
                    let ev = self.builder.build_load(elem_llvm, sp, "ev").unwrap();
                    let cv = self.builder.build_call(cf, &[ev.into()], "cv").unwrap().try_as_basic_value().basic().unwrap();
                    let dp = unsafe { self.builder.build_in_bounds_gep(elem_llvm, dst, &[i], "dp").unwrap() };
                    self.builder.build_store(dp, cv).unwrap();
                    let nx = self.builder.build_int_add(i, self.i64t().const_int(1, false), "nx").unwrap();
                    phi.add_incoming(&[(&nx, self.builder.get_insert_block().unwrap())]);
                    self.builder.build_unconditional_branch(head).unwrap();
                    self.builder.position_at_end(exit);
                } else {
                    let bytes = self.builder.build_int_mul(len, elem_size, "cb").unwrap();
                    self.memcpy_bytes(dst, src, bytes);
                }
                self.make_len_ptr(len, dst).into()
            }
            ZType::Tuple(elems) => {
                let sv = v.into_struct_value();
                let mut cur = self.types.llvm(zt).into_struct_type().get_undef();
                for (i, e) in elems.iter().enumerate() {
                    let fv = self.builder.build_extract_value(sv, i as u32, "tf").unwrap();
                    let cv = match self.get_or_build_clone(e) {
                        Some(cf) => self.builder.build_call(cf, &[fv.into()], "cv").unwrap().try_as_basic_value().basic().unwrap(),
                        None => fv,
                    };
                    cur = self.builder.build_insert_value(cur, cv, i as u32, "ti").unwrap().into_struct_value();
                }
                cur.into()
            }
            ZType::Struct(name) => {
                let fields = self.types.structs.borrow()[name].fields.clone();
                let sv = v.into_struct_value();
                let mut cur = self.types.struct_llvm(name).get_undef();
                for (i, (_, ft)) in fields.iter().enumerate() {
                    let fv = self.builder.build_extract_value(sv, i as u32, "sf").unwrap();
                    let cv = match self.get_or_build_clone(ft) {
                        Some(cf) => self.builder.build_call(cf, &[fv.into()], "cv").unwrap().try_as_basic_value().basic().unwrap(),
                        None => fv,
                    };
                    cur = self.builder.build_insert_value(cur, cv, i as u32, "si").unwrap().into_struct_value();
                }
                cur.into()
            }
            // Deep-copy the active variant's heap so the clone owns it independently:
            // switch on the tag, clone the payload (Str/Array buffer; Struct → a fresh
            // box), and overwrite (p0, p1). Inline payloads copy with the value as-is.
            ZType::Enum(name) => {
                let variants = self.types.enums.borrow()[name].clone();
                let sv = v.into_struct_value();
                let tag = self.builder.build_extract_value(sv, 0, "etag").unwrap().into_int_value();
                let p0 = self.builder.build_extract_value(sv, 1, "ep0").unwrap().into_int_value();
                let p1 = self.builder.build_extract_value(sv, 2, "ep1").unwrap().into_pointer_value();
                let enum_ty = self.types.llvm(zt);
                let slot = self.builder.build_alloca(enum_ty, "eclone").unwrap();
                self.builder.build_store(slot, v).unwrap();
                let end = self.context.append_basic_block(func, "ec.end");
                let default = self.context.append_basic_block(func, "ec.def");
                let mut cases = Vec::new();
                let mut managed = Vec::new();
                for (i, (_, payload)) in variants.iter().enumerate() {
                    let pt = match payload {
                        Some(pt) if self.needs_drop(pt) || matches!(pt, ZType::Struct(_)) => pt.clone(),
                        _ => continue,
                    };
                    let blk = self.context.append_basic_block(func, "ec.case");
                    cases.push((self.i64t().const_int(i as u64, false), blk));
                    managed.push((blk, pt));
                }
                self.builder.build_switch(tag, default, &cases).unwrap();
                for (blk, pt) in managed {
                    self.builder.position_at_end(blk);
                    let (np0, np1): (IntValue<'ctx>, PointerValue<'ctx>) = match &pt {
                        ZType::Str | ZType::Array(_) => {
                            let payload = self.make_len_ptr(p0, p1);
                            let cloned = self.clone_value(payload.into(), &pt).into_struct_value();
                            self.len_ptr_parts(cloned)
                        }
                        ZType::Struct(sname) => {
                            let struct_ty = self.types.struct_llvm(sname);
                            let loaded = self.builder.build_load(struct_ty, p1, "boxload").unwrap();
                            let cloned = self.clone_value(loaded, &pt);
                            let size = struct_ty.size_of().unwrap();
                            let boxed = self.malloc_bytes(size);
                            self.builder.build_store(boxed, cloned).unwrap();
                            (self.i64t().const_zero(), boxed)
                        }
                        _ => (p0, p1),
                    };
                    let f1 = self.builder.build_struct_gep(enum_ty, slot, 1, "ecf1").unwrap();
                    self.builder.build_store(f1, np0).unwrap();
                    let f2 = self.builder.build_struct_gep(enum_ty, slot, 2, "ecf2").unwrap();
                    self.builder.build_store(f2, np1).unwrap();
                    self.builder.build_unconditional_branch(end).unwrap();
                }
                self.builder.position_at_end(default);
                self.builder.build_unconditional_branch(end).unwrap();
                self.builder.position_at_end(end);
                self.builder.build_load(enum_ty, slot, "eres").unwrap()
            }
            // Deep-copy via the per-lambda clone thunk (mallocs a fresh env, clones
            // captures). fn/drop/clone pointers carry over; only env is duplicated.
            ZType::Closure(_, _) => {
                let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                let sv = v.into_struct_value();
                let env = self.builder.build_extract_value(sv, 1, "cenv").unwrap().into_pointer_value();
                let thunk = self.builder.build_extract_value(sv, 3, "cclone").unwrap().into_pointer_value();
                let is_null = self.builder.build_is_null(thunk, "tn").unwrap();
                let call_blk = self.context.append_basic_block(func, "cc.call");
                let null_blk = self.context.append_basic_block(func, "cc.null");
                let merge = self.context.append_basic_block(func, "cc.merge");
                self.builder.build_conditional_branch(is_null, null_blk, call_blk).unwrap();
                self.builder.position_at_end(call_blk);
                let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
                let ne = self.builder.build_indirect_call(fn_ty, thunk, &[env.into()], "ce").unwrap()
                    .try_as_basic_value().basic().unwrap().into_pointer_value();
                self.builder.build_unconditional_branch(merge).unwrap();
                let call_end = self.builder.get_insert_block().unwrap();
                self.builder.position_at_end(null_blk);
                self.builder.build_unconditional_branch(merge).unwrap();
                self.builder.position_at_end(merge);
                let phi = self.builder.build_phi(ptr_ty, "newenv").unwrap();
                phi.add_incoming(&[(&ne, call_end), (&env, null_blk)]);
                let new_env = phi.as_basic_value().into_pointer_value();
                let r = self.builder.build_insert_value(sv, new_env, 1, "cl1").unwrap();
                r.into_struct_value().into()
            }
            _ => v,
        }
    }

    /// Recursively clone a value (call `@__clone_T`) so it uniquely owns its heap.
    fn clone_value(&self, value: BasicValueEnum<'ctx>, zt: &ZType) -> BasicValueEnum<'ctx> {
        match self.get_or_build_clone(zt) {
            Some(f) => self.builder.build_call(f, &[value.into()], "clone").unwrap().try_as_basic_value().basic().unwrap(),
            None => value,
        }
    }

    /// Recursively drop the value currently held in `slot` (call `@__drop_T`).
    fn drop_local(&self, slot: PointerValue<'ctx>, zt: &ZType) {
        if let Some(f) = self.get_or_build_drop(zt) {
            let val = self.builder.build_load(self.types.llvm(zt), slot, "dl").unwrap();
            self.builder.build_call(f, &[val.into()], "").unwrap();
        }
    }

    /// Drop the managed locals declared since the `saved` scope snapshot (the
    /// Rust-style scope-exit `Drop`). Called only on fall-through; value semantics
    /// gives each local unique ownership, so the drop is sound (no double-free /
    /// use-after-free).
    fn free_scope_locals(&mut self, saved: &HashMap<String, (PointerValue<'ctx>, ZType)>) {
        let mut targets: Vec<(String, PointerValue<'ctx>, ZType)> = Vec::new();
        for (name, (slot, zt)) in self.locals.iter() {
            if !self.needs_drop(zt) {
                continue;
            }
            let outer = saved.get(name).map(|(s, _)| *s == *slot).unwrap_or(false);
            if !outer {
                targets.push((name.clone(), *slot, zt.clone()));
            }
        }
        for (name, slot, zt) in targets {
            self.drop_local_guarded(&name, slot, &zt);
        }
    }

    /// Drop every currently-live managed local except `skip` (the slot whose
    /// value MOVES out as the return value). Sound because the function is
    /// exiting so every other managed local is dead.
    fn free_live_managed_except(&self, skip: Option<PointerValue<'ctx>>) {
        let targets: Vec<(String, PointerValue<'ctx>, ZType)> = self
            .locals
            .iter()
            .filter(|(_, (slot, zt))| self.needs_drop(zt) && Some(*slot) != skip)
            .map(|(name, (slot, zt))| (name.clone(), *slot, zt.clone()))
            .collect();
        for (name, slot, zt) in targets {
            self.drop_local_guarded(&name, slot, &zt);
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
            MirStmt::Local { name, value, ty, .. } => {
                // Lower the initializer FIRST (so `let x = x + 1` reads the outer
                // `x`), then allocate a fresh slot typed by the value and bind it —
                // shadowing any outer binding until this scope ends.
                let (v, mut vt) = self.lower_expr(value)?;
                // A `*T` annotation refines a raw pointer's pointee: `ptr_from_addr`
                // yields a default `*Int`, but `let p: *Point = ...` must type `p`
                // as `*Point` so `ptr_read(p)` loads a `Point`. The value's LLVM
                // repr (an opaque `ptr`) is identical, so only the ZType changes.
                if let (ZType::Ptr(_), Some(ann)) = (&vt, ty) {
                    if let Ok(resolved @ ZType::Ptr(_)) = self.types.resolve_ann_ztype(ann) {
                        vt = resolved;
                    }
                }
                let v = self.bind_owned(value, v, &vt);
                let slot = self.entry_alloca(name, self.types.llvm(&vt));
                self.builder.build_store(slot, v).unwrap();
                self.locals.insert(name.clone(), (slot, vt));
                // Fresh binding starts owned: (re)zero its moved-flag if flagged.
                self.bind_moved_flag(name);
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
                // Reassigning a simple managed local: drop the old value first. The
                // new value was already cloned/owned by `bind_owned`, so it can't
                // alias the old, and the old is uniquely owned by this local —
                // unless it was already moved out, which the flag-guarded drop
                // skips. After the store the local owns a fresh value again.
                if let MirPlace::Local(name) = place {
                    if self.needs_drop(&slot_ty) {
                        self.drop_local_guarded(name, slot, &slot_ty);
                    }
                    self.builder.build_store(slot, v).unwrap();
                    self.bind_moved_flag(name);
                    return Ok(false);
                }
                self.builder.build_store(slot, v).unwrap();
                Ok(false)
            }
            MirStmt::Return(value) => {
                let v = match value {
                    Some(expr) => {
                        let (v0, vt) = self.lower_expr(expr)?;
                        if !self.needs_drop(&vt) {
                            self.free_live_managed_except(None);
                            v0
                        } else if let Some(slot) = match expr {
                            // Returning a whole managed local → MOVE it: keep its
                            // value, drop every OTHER managed local.
                            MirExpr::Load(name) => self
                                .locals
                                .get(name)
                                .and_then(|(slot, zt)| self.needs_drop(zt).then_some(*slot)),
                            _ => None,
                        } {
                            self.free_live_managed_except(Some(slot));
                            v0
                        } else {
                            // Owned literal/call (independent) or a field/element
                            // that ALIASES a local: make the return value
                            // independent (clone the aliasing case), THEN drop all.
                            let v = self.bind_owned(expr, v0, &vt);
                            self.free_live_managed_except(None);
                            v
                        }
                    }
                    None => {
                        self.free_live_managed_except(None);
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
        // A String scrutinee can't drive an integer `switch`; lower it as a
        // sequential chain of `string_eq` tests (first match wins).
        if vty == ZType::Str {
            return self.lower_string_match(val, arms);
        }
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
                            // Reconstruct the String {len, ptr} from (p0, p1), then
                            // CLONE it so the binding owns an independent buffer — the
                            // enum still owns the original and frees it on drop, so a
                            // shared pointer would double-free.
                            let p0 = self.builder.build_extract_value(sv, 1, "plen").unwrap().into_int_value();
                            let p1 = self.builder.build_extract_value(sv, 2, "pdata").unwrap().into_pointer_value();
                            let shared = self.make_len_ptr(p0, p1).into();
                            (self.clone_value(shared, &ZType::Str), ZType::Str)
                        }
                        Some(ZType::Array(elem)) => {
                            // Rebuild {len, ptr} from (p0, p1), then element-deep clone
                            // so the binding is an independent owner (its element
                            // strings/arrays don't alias the enum's — both get dropped).
                            let p0 = self.builder.build_extract_value(sv, 1, "plen").unwrap().into_int_value();
                            let p1 = self.builder.build_extract_value(sv, 2, "pdata").unwrap().into_pointer_value();
                            let shared = self.make_len_ptr(p0, p1).into();
                            let ty = ZType::Array(elem);
                            (self.clone_value(shared, &ty), ty)
                        }
                        Some(ZType::Struct(name)) => {
                            // p1 points at the boxed struct; load it, then clone so the
                            // binding's managed fields don't alias the box (the enum
                            // frees the box + its fields on drop).
                            let p1 = self.builder.build_extract_value(sv, 2, "pbox").unwrap().into_pointer_value();
                            let ty = ZType::Struct(name);
                            let struct_ty = self.types.llvm(&ty);
                            let loaded = self.builder.build_load(struct_ty, p1, "boxload").unwrap();
                            (self.clone_value(loaded, &ty), ty)
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

    /// Lower a `match` on a String scrutinee as a sequential chain of `string_eq`
    /// tests — `match s { "a" -> .., "b" -> .., _ -> .. }`. Each String arm tests
    /// equality and branches to its body or the next test; the catch-all
    /// (Name/Wildcard, required for exhaustiveness since strings are unbounded)
    /// terminates the chain. Mirrors the interpreter's first-match-wins order.
    fn lower_string_match(
        &mut self,
        sval: BasicValueEnum<'ctx>,
        arms: &[crate::mir::MirMatchArm],
    ) -> Result<bool, String> {
        let scrutinee = sval.into_struct_value();
        let end_bb = self.context.append_basic_block(self.llvm_fn, "smatch.end");
        // `cur` is the block where the next arm's test is emitted.
        let mut cur = self.builder.get_insert_block().unwrap();
        let mut fell_through = true;
        for arm in arms {
            self.builder.position_at_end(cur);
            match &arm.pattern {
                MirPattern::String(text) => {
                    let (lit, _) = self.lower_expr(&MirExpr::String(text.clone()))?;
                    let eq = self.string_eq(scrutinee, lit.into_struct_value());
                    let arm_bb = self.context.append_basic_block(self.llvm_fn, "smatch.arm");
                    let next_bb = self.context.append_basic_block(self.llvm_fn, "smatch.next");
                    self.builder.build_conditional_branch(eq, arm_bb, next_bb).unwrap();
                    self.builder.position_at_end(arm_bb);
                    let scope = self.locals.clone();
                    if !self.lower_stmts(&arm.body)? {
                        self.builder.build_unconditional_branch(end_bb).unwrap();
                    }
                    self.locals = scope;
                    cur = next_bb;
                }
                MirPattern::Name(name) => {
                    // Catch-all binding: owns a clone of the scrutinee string.
                    let scope = self.locals.clone();
                    let cloned = self.clone_value(sval, &ZType::Str);
                    let slot = self.entry_alloca(name, self.types.llvm(&ZType::Str));
                    self.builder.build_store(slot, cloned).unwrap();
                    self.locals.insert(name.clone(), (slot, ZType::Str));
                    if !self.lower_stmts(&arm.body)? {
                        self.builder.build_unconditional_branch(end_bb).unwrap();
                    }
                    self.locals = scope;
                    fell_through = false;
                    break;
                }
                MirPattern::Wildcard => {
                    let scope = self.locals.clone();
                    if !self.lower_stmts(&arm.body)? {
                        self.builder.build_unconditional_branch(end_bb).unwrap();
                    }
                    self.locals = scope;
                    fell_through = false;
                    break;
                }
                _ => return Err("non-string pattern in a string match".into()),
            }
        }
        // No catch-all (should be unreachable — typecheck requires exhaustiveness).
        if fell_through {
            self.builder.position_at_end(cur);
            self.builder.build_unreachable().unwrap();
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
                // Copy-on-write: writing an element mutates the buffer in place, so
                // make it uniquely owned first (a no-op when not shared), then GEP.
                let (_, data) = self.cow_make_unique(base_slot, &elem);
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
                trait_methods: self.trait_methods,
                specialized: self.specialized,
                malloc: self.malloc,
                free: self.free,
                memcpy: self.memcpy,
                memcmp: self.memcmp,
                llvm_fn: lifted,
                entry_bb: lifted_entry,
                lambda_count: 0,
                locals: HashMap::new(),
                move_plan: crate::move_analysis::MovePlan::empty(),
                moved_flags: HashMap::new(),
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

        // Allocate the environment and CLONE captured values into it, so the env
        // owns its heap independently of the enclosing locals (both get dropped —
        // a shared buffer would double-free). Non-managed captures clone to a copy.
        let env_size = env_ty
            .size_of()
            .ok_or_else(|| "env type has no size".to_string())?;
        let env_mem = self.malloc_bytes(env_size);
        for (index, (cap_name, cap_ty, slot)) in captures.iter().enumerate() {
            let llvm_ty = self.types.llvm(cap_ty);
            let val = self.builder.build_load(llvm_ty, *slot, cap_name).unwrap();
            let owned = self.clone_value(val, cap_ty);
            let field_ptr = self
                .builder
                .build_struct_gep(env_ty, env_mem, index as u32, "capst")
                .map_err(|_| "env store GEP failed".to_string())?;
            self.builder.build_store(field_ptr, owned).unwrap();
        }

        // Per-lambda env thunks (carry the capture layout for type-level drop/clone).
        let drop_thunk = self.build_env_drop_thunk(&name, env_ty, &captures)?;
        let clone_thunk = self.build_env_clone_thunk(&name, env_ty, env_size, &captures)?;

        // Build the closure value { fn_ptr, env_ptr, drop_thunk, clone_thunk }.
        let clo_ty = closure_struct_type(self.context);
        let fn_ptr = lifted.as_global_value().as_pointer_value();
        let clo = self
            .builder
            .build_insert_value(clo_ty.get_undef(), fn_ptr, 0, "clo_fn")
            .unwrap();
        let clo = self
            .builder
            .build_insert_value(clo, env_mem, 1, "clo_env")
            .unwrap();
        let clo = self
            .builder
            .build_insert_value(clo, drop_thunk, 2, "clo_drop")
            .unwrap();
        let clo = self
            .builder
            .build_insert_value(clo, clone_thunk, 3, "clo_clone")
            .unwrap()
            .into_struct_value();
        Ok((clo.into(), ZType::Closure(param_ztys, Box::new(ret_zty))))
    }

    /// Generate `void @<lambda>_dropenv(ptr env)`: drop each managed capture, then
    /// free the environment buffer. Returned as a function pointer for the closure
    /// value's drop-thunk slot.
    fn build_env_drop_thunk(
        &self,
        lambda: &str,
        env_ty: StructType<'ctx>,
        captures: &[(String, ZType, PointerValue<'ctx>)],
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let fn_ty = self.context.void_type().fn_type(&[ptr_ty.into()], false);
        let func = self.module.add_function(&format!("{lambda}_dropenv"), fn_ty, None);
        let saved = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        let env = func.get_nth_param(0).unwrap().into_pointer_value();
        for (index, (_, cap_ty, _)) in captures.iter().enumerate() {
            if let Some(df) = self.get_or_build_drop(cap_ty) {
                let fp = self.builder.build_struct_gep(env_ty, env, index as u32, "df").unwrap();
                let v = self.builder.build_load(self.types.llvm(cap_ty), fp, "dv").unwrap();
                self.builder.build_call(df, &[v.into()], "").unwrap();
            }
        }
        self.builder.build_call(self.free, &[env.into()], "").unwrap();
        self.builder.build_return(None).unwrap();
        if let Some(b) = saved {
            self.builder.position_at_end(b);
        }
        Ok(func.as_global_value().as_pointer_value())
    }

    /// Generate `ptr @<lambda>_cloneenv(ptr env)`: malloc a fresh environment and
    /// deep-clone each capture into it; return the new env pointer.
    fn build_env_clone_thunk(
        &self,
        lambda: &str,
        env_ty: StructType<'ctx>,
        env_size: IntValue<'ctx>,
        captures: &[(String, ZType, PointerValue<'ctx>)],
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let fn_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
        let func = self.module.add_function(&format!("{lambda}_cloneenv"), fn_ty, None);
        let saved = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        let old = func.get_nth_param(0).unwrap().into_pointer_value();
        let new_env = self.malloc_bytes(env_size);
        for (index, (_, cap_ty, _)) in captures.iter().enumerate() {
            let llvm_ty = self.types.llvm(cap_ty);
            let ofp = self.builder.build_struct_gep(env_ty, old, index as u32, "cof").unwrap();
            let v = self.builder.build_load(llvm_ty, ofp, "cov").unwrap();
            let cloned = self.clone_value(v, cap_ty);
            let nfp = self.builder.build_struct_gep(env_ty, new_env, index as u32, "cnf").unwrap();
            self.builder.build_store(nfp, cloned).unwrap();
        }
        self.builder.build_return(Some(&new_env)).unwrap();
        if let Some(b) = saved {
            self.builder.position_at_end(b);
        }
        Ok(func.as_global_value().as_pointer_value())
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
    /// Lower a trait-method call `m(recv, ..)`: lower the receiver to learn its
    /// concrete ZType, route to the flattened impl `m$<TypeBase>` (a concrete
    /// function), and call it with the receiver reused as the first argument.
    fn lower_trait_dispatch_call(
        &mut self,
        callee: &str,
        args: &[MirExpr],
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let (recv_v, recv_t) = self.lower_expr(&args[0])?;
        let base = zty_base_name(&recv_t);
        let mangled = crate::type_syntax::dispatch_name(callee, &base);
        let function = self.functions.get(&mangled).copied().ok_or_else(|| {
            format!("no implementation of trait method `{callee}` for type `{base}`")
        })?;
        let mut argv = Vec::with_capacity(args.len());
        argv.push(self.bind_owned(&args[0], recv_v, &recv_t).into());
        for arg in &args[1..] {
            let (v, vt) = self.lower_expr(arg)?;
            argv.push(self.bind_owned(arg, v, &vt).into());
        }
        let call = self.builder.build_call(function, &argv, "tcall").unwrap();
        let ret = self.types.returns.get(&mangled).cloned().unwrap_or(ZType::Int);
        let value = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{mangled}` returned no value"))?;
        Ok((value, ret))
    }

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
        {
            let instances = self.types.instances.borrow();
            for (param, arg_zty) in generic.params.iter().zip(&arg_ztys) {
                unify_ztype(&param.ty, arg_zty, &generic.type_params, &instances, &mut subst);
            }
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
            if base == "Array" && arg_strs.len() == 1 {
                return Ok(ZType::Array(Box::new(
                    self.resolve_generic_ztype(arg_strs[0], subst)?,
                )));
            }
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
                trait_methods: self.trait_methods,
                specialized: self.specialized,
                malloc: self.malloc,
                free: self.free,
                memcpy: self.memcpy,
                memcmp: self.memcmp,
                llvm_fn: func,
                entry_bb: entry,
                lambda_count: 0,
                locals: HashMap::new(),
                move_plan: crate::move_analysis::analyze(generic),
                moved_flags: HashMap::new(),
                loops: Vec::new(),
            };
            for (index, (param, zt)) in generic.params.iter().zip(param_ztys).enumerate() {
                let slot = inner.entry_alloca(&param.name, inner.types.llvm(zt));
                let value = func.get_nth_param(index as u32).expect("param exists");
                inner.builder.build_store(slot, value).unwrap();
                inner.locals.insert(param.name.clone(), (slot, zt.clone()));
                inner.bind_moved_flag(&param.name);
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
                // A user-defined function shadows a same-named std builtin (lets the
                // bare-metal kernel define its own `fn print` over a UART instead of
                // the std.io libc-`write` one). Only shadows when actually defined,
                // so true builtins — and fixpoint — are unaffected.
                if !self.functions.contains_key(callee) {
                    if let Some(result) = self.lower_builtin(callee, args)? {
                        return Ok(result);
                    }
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
                // Trait-method UFCS dispatch: route `m(recv, ..)` to the flattened
                // impl `m$<TypeBase>` selected by the receiver's static ZType.
                if self.trait_methods.contains(callee) && !args.is_empty() {
                    return self.lower_trait_dispatch_call(callee, args);
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
                        unify_ztype(
                            fty,
                            &vt,
                            &type_params,
                            &self.types.instances.borrow(),
                            &mut subst,
                        );
                        // The struct owns each managed field.
                        let v = self.bind_owned(value_expr, v, &vt);
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
                    let (v, vt) = self.lower_expr(value_expr)?;
                    // The struct owns each managed field.
                    let v = self.bind_owned(value_expr, v, &vt);
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
                for (index, (element, (v, vt))) in elements.iter().zip(values).enumerate() {
                    // The tuple owns each managed element.
                    let v = self.bind_owned(element, v, &vt);
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
                for (i, (element, (v, vt))) in elements.iter().zip(values).enumerate() {
                    // The array owns its elements: clone a managed, non-fresh element.
                    let v = self.bind_owned(element, v, &vt);
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
                // Immutable bytes → a private global `{ i64 STATIC_RC, [bytes, NUL] }`
                // mirroring the heap string layout (refcount header + data), so the
                // SAME clone/drop refcount path handles literals: the sentinel rc makes
                // them never bump/free. The value is `{ byte_len, ptr-to-bytes }`; `len`
                // excludes the NUL (matches the interpreter's byte count).
                let i8t = self.context.i8_type();
                let mut bytes: Vec<_> = text.bytes().map(|byte| i8t.const_int(byte as u64, false)).collect();
                bytes.push(i8t.const_zero());
                let byte_arr = i8t.const_array(&bytes);
                let header = self.i64t().const_int(STATIC_STR_RC, false);
                let init = self.context.const_struct(&[header.into(), byte_arr.into()], false);
                let global = self.module.add_global(init.get_type(), None, "str");
                global.set_initializer(&init);
                global.set_constant(true);
                global.set_linkage(inkwell::module::Linkage::Private);
                let data = self
                    .builder
                    .build_struct_gep(init.get_type(), global.as_pointer_value(), 1, "strbytes")
                    .unwrap();
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
                        let (v0, vt) = self.lower_expr(expr)?;
                        if let Some(decl) = &payload_decl {
                            unify_ztype(
                                decl,
                                &vt,
                                &type_params,
                                &self.types.instances.borrow(),
                                &mut subst,
                            );
                        }
                        // The enum owns its payload, independent of any droppable
                        // source it aliases. The enum value itself is NOT dropped
                        // (conservative leak for now), but cloning keeps the source
                        // safely droppable. `bind_owned` skips the clone for an
                        // already-owned source (a fresh call result).
                        let v = self.bind_owned(expr, v0, &vt);
                        match &vt {
                            ZType::Int => (v.into_int_value(), null),
                            ZType::Str | ZType::Array(_) => {
                                let (len, data) = self.len_ptr_parts(v.into_struct_value());
                                (len, data)
                            }
                            ZType::Struct(name) => {
                                // Struct payload is wider than the inline slot: box the
                                // (already-owned) struct on the heap, pointer in p1.
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
                let buf = self.alloc_str_buf(total);
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
                let buf = self.alloc_str_buf(len);
                self.memcpy_bytes(buf, src, len);
                Ok(Some((self.make_len_ptr(len, buf).into(), ZType::Str)))
            }
            "int_to_string" => {
                // Self-contained decimal conversion (no libc snprintf) — see
                // `gen_int_to_string`. Keeps the native backend libc-free.
                let n = self.lower_int(&args[0])?;
                Ok(Some((self.gen_int_to_string(n).into(), ZType::Str)))
            }
            "int_abs" => {
                let n = self.lower_int(&args[0])?;
                let neg = b.build_int_neg(n, "neg").unwrap();
                let isneg = b
                    .build_int_compare(IntPredicate::SLT, n, self.i64t().const_zero(), "isneg")
                    .unwrap();
                let r = b.build_select(isneg, neg, n, "abs").unwrap().into_int_value();
                Ok(Some((r.into(), ZType::Int)))
            }
            "int_min" => {
                let a = self.lower_int(&args[0])?;
                let c = self.lower_int(&args[1])?;
                let lt = b.build_int_compare(IntPredicate::SLT, a, c, "lt").unwrap();
                let r = b.build_select(lt, a, c, "min").unwrap().into_int_value();
                Ok(Some((r.into(), ZType::Int)))
            }
            "int_max" => {
                let a = self.lower_int(&args[0])?;
                let c = self.lower_int(&args[1])?;
                let gt = b.build_int_compare(IntPredicate::SGT, a, c, "gt").unwrap();
                let r = b.build_select(gt, a, c, "max").unwrap().into_int_value();
                Ok(Some((r.into(), ZType::Int)))
            }
            "int_pow" => {
                let base = self.lower_int(&args[0])?;
                let exp = self.lower_int(&args[1])?;
                Ok(Some((self.gen_int_pow(base, exp).into(), ZType::Int)))
            }
            "string_to_int" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let r = self.gen_string_to_int(s.into_struct_value());
                Ok(Some((r.into(), ZType::Int)))
            }
            "string_index_of" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let (sub, _) = self.lower_expr(&args[1])?;
                let idx = self.gen_index_of(s.into_struct_value(), sub.into_struct_value());
                Ok(Some((idx.into(), ZType::Int)))
            }
            "string_contains" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let (sub, _) = self.lower_expr(&args[1])?;
                let idx = self.gen_index_of(s.into_struct_value(), sub.into_struct_value());
                let found = self
                    .builder
                    .build_int_compare(IntPredicate::SGE, idx, self.i64t().const_zero(), "found")
                    .unwrap();
                Ok(Some((self.bool_to_i64(found).into(), ZType::Int)))
            }
            "string_repeat" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let n = self.lower_int(&args[1])?;
                let v = self.gen_repeat(s.into_struct_value(), n);
                Ok(Some((v.into(), ZType::Str)))
            }
            "string_to_upper" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let v = self.gen_case_map(s.into_struct_value(), false);
                Ok(Some((v.into(), ZType::Str)))
            }
            "string_to_lower" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let v = self.gen_case_map(s.into_struct_value(), true);
                Ok(Some((v.into(), ZType::Str)))
            }
            "string_trim" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let v = self.gen_trim(s.into_struct_value());
                Ok(Some((v.into(), ZType::Str)))
            }
            // stdout output via libc `write(1, data, len)`. The JIT/AOT resolves
            // `write` from libc; hosted-only (the freestanding kernel uses its own
            // UART `print`, never this std.io builtin).
            "print" | "println" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let (len, data) = self.len_ptr_parts(s.into_struct_value());
                let write = self.module.get_function("write").unwrap_or_else(|| {
                    let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                    let fnty = self
                        .i64t()
                        .fn_type(&[self.context.i32_type().into(), ptr_ty.into(), self.i64t().into()], false);
                    self.module.add_function("write", fnty, None)
                });
                let fd = self.context.i32_type().const_int(1, false); // stdout
                b.build_call(write, &[fd.into(), data.into(), len.into()], "").unwrap();
                if callee == "println" {
                    let nl = b.build_global_string_ptr("\n", "nl").unwrap().as_pointer_value();
                    let one = self.i64t().const_int(1, false);
                    b.build_call(write, &[fd.into(), nl.into(), one.into()], "").unwrap();
                }
                Ok(Some((self.i64t().const_zero().into(), ZType::Int)))
            }
            // Volatile memory-mapped writes at the given width (device registers /
            // page-table entries the optimizer must not drop or reorder).
            "mmio_write_byte" | "mmio_write_word" | "mmio_write_dword" => {
                let bits = mmio_bits(callee);
                let addr = self.lower_int(&args[0])?;
                let value = self.lower_int(&args[1])?;
                let ptr = b
                    .build_int_to_ptr(addr, self.context.ptr_type(inkwell::AddressSpace::default()), "mmiop")
                    .unwrap();
                // The i64 value carries the low `bits`; narrow it for sub-64 widths.
                let narrowed = if bits == 64 {
                    value
                } else {
                    b.build_int_truncate(value, self.context.custom_width_int_type(std::num::NonZeroU32::new(bits).unwrap()).unwrap(), "mmiov").unwrap()
                };
                let store = b.build_store(ptr, narrowed).unwrap();
                store.set_volatile(true).unwrap();
                Ok(Some((self.i64t().const_zero().into(), ZType::Int)))
            }
            // Volatile memory-mapped reads, zero-extended to the i64 value repr.
            "mmio_read_byte" | "mmio_read_word" | "mmio_read_dword" => {
                let bits = mmio_bits(callee);
                let addr = self.lower_int(&args[0])?;
                let width = self.context.custom_width_int_type(std::num::NonZeroU32::new(bits).unwrap()).unwrap();
                let ptr = b
                    .build_int_to_ptr(addr, self.context.ptr_type(inkwell::AddressSpace::default()), "mmiop")
                    .unwrap();
                let load = b.build_load(width, ptr, "mmior").unwrap().into_int_value();
                load.as_instruction().unwrap().set_volatile(true).unwrap();
                let widened = if bits == 64 {
                    load
                } else {
                    b.build_int_z_extend(load, self.i64t(), "mmiow").unwrap()
                };
                Ok(Some((widened.into(), ZType::Int)))
            }
            // Raw pointers (unsafe, native-only). A `*T` is an LLVM opaque `ptr`;
            // the pointee ZType drives load/store width and offset stride.
            "ptr_from_addr" => {
                // Int address → `*T`. The element defaults to Int here; the
                // binding's annotation (a `*T` let/param) refines the slot type,
                // which is what `ptr_read`/`ptr_write` consult for the width.
                let addr = self.lower_int(&args[0])?;
                let ptr = b
                    .build_int_to_ptr(addr, self.context.ptr_type(inkwell::AddressSpace::default()), "pfa")
                    .unwrap();
                Ok(Some((ptr.into(), ZType::Ptr(Box::new(ZType::Int)))))
            }
            "ptr_addr" => {
                let (pv, _) = self.lower_expr(&args[0])?;
                let addr = b
                    .build_ptr_to_int(pv.into_pointer_value(), self.i64t(), "pta")
                    .unwrap();
                Ok(Some((addr.into(), ZType::Int)))
            }
            "ptr_read" => {
                let (pv, pt) = self.lower_expr(&args[0])?;
                let elem = match pt {
                    ZType::Ptr(e) => *e,
                    _ => ZType::Int,
                };
                let val = b
                    .build_load(self.types.llvm(&elem), pv.into_pointer_value(), "pread")
                    .unwrap();
                Ok(Some((val, elem)))
            }
            "ptr_write" => {
                let (pv, _) = self.lower_expr(&args[0])?;
                let (val, _) = self.lower_expr(&args[1])?;
                b.build_store(pv.into_pointer_value(), val).unwrap();
                Ok(Some((self.i64t().const_zero().into(), ZType::Int)))
            }
            // Inline-assembly privileged ops (riscv-only). The CSR is baked into
            // the instruction as a literal, so it must be an Int literal here.
            "csr_read" => {
                let csr = self.const_csr(&args[0])?;
                let fn_ty = self.i64t().fn_type(&[], false);
                let asm = self.context.create_inline_asm(
                    fn_ty,
                    format!("csrr $0, {csr}"),
                    "=r".to_string(),
                    true,
                    false,
                    None,
                    false,
                );
                let val = b
                    .build_indirect_call(fn_ty, asm, &[], "csrr")
                    .unwrap()
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                Ok(Some((val, ZType::Int)))
            }
            "csr_write" | "csr_set" | "csr_clear" => {
                let csr = self.const_csr(&args[0])?;
                let val = self.lower_int(&args[1])?;
                let mnem = match callee {
                    "csr_write" => "csrw",
                    "csr_set" => "csrs",
                    _ => "csrc",
                };
                let fn_ty = self.context.void_type().fn_type(&[self.i64t().into()], false);
                let asm = self.context.create_inline_asm(
                    fn_ty,
                    format!("{mnem} {csr}, $0"),
                    "r".to_string(),
                    true,
                    false,
                    None,
                    false,
                );
                b.build_indirect_call(fn_ty, asm, &[val.into()], "").unwrap();
                Ok(Some((self.i64t().const_zero().into(), ZType::Int)))
            }
            "wfi" => {
                let fn_ty = self.context.void_type().fn_type(&[], false);
                let asm = self.context.create_inline_asm(
                    fn_ty,
                    "wfi".to_string(),
                    String::new(),
                    true,
                    false,
                    None,
                    false,
                );
                b.build_indirect_call(fn_ty, asm, &[], "").unwrap();
                Ok(Some((self.i64t().const_zero().into(), ZType::Int)))
            }
            "array_data_addr" => {
                // The data pointer of an array's `{len, ptr}` value, as an Int —
                // for raw access or handing a buffer to hardware.
                let (a, _) = self.lower_expr(&args[0])?;
                let data = b
                    .build_extract_value(a.into_struct_value(), 1, "adata")
                    .unwrap()
                    .into_pointer_value();
                let addr = b.build_ptr_to_int(data, self.i64t(), "aaddr").unwrap();
                Ok(Some((addr.into(), ZType::Int)))
            }
            "ptr_offset" => {
                // p + count*sizeof(T), via ptrtoint/add/inttoptr (no GEP needed).
                let (pv, pt) = self.lower_expr(&args[0])?;
                let count = self.lower_int(&args[1])?;
                let elem = match &pt {
                    ZType::Ptr(e) => (**e).clone(),
                    _ => ZType::Int,
                };
                let stride = self.types.llvm(&elem).size_of().unwrap();
                let base = b.build_ptr_to_int(pv.into_pointer_value(), self.i64t(), "po.base").unwrap();
                let delta = b.build_int_mul(count, stride, "po.delta").unwrap();
                let sum = b.build_int_add(base, delta, "po.sum").unwrap();
                let ptr = b
                    .build_int_to_ptr(sum, self.context.ptr_type(inkwell::AddressSpace::default()), "po.ptr")
                    .unwrap();
                Ok(Some((ptr.into(), pt)))
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
            // Generic array intrinsics: the element ZType is read from the lowered
            // argument (the array for push, the seed for repeat), so they work for
            // any monomorphized element type.
            "array_push" => {
                let (arr, arr_zt) = self.lower_expr(&args[0])?;
                let ZType::Array(elem) = arr_zt else {
                    return Err("array_push expects an array first argument".into());
                };
                Ok(Some(self.lower_array_push_with(arr, &args[1], *elem)?))
            }
            "array_repeat" => Ok(Some(self.lower_array_repeat(&args[0], &args[1])?)),
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
        self.lower_array_push_with(arr, value_expr, elem)
    }

    /// Functional push given an already-lowered array value (used by the generic
    /// `array_push`, which lowers the array once to learn its element ZType).
    fn lower_array_push_with(
        &mut self,
        arr: BasicValueEnum<'ctx>,
        value_expr: &MirExpr,
        elem: ZType,
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let (len, data) = self.len_ptr_parts(arr.into_struct_value());
        let (x0, _) = self.lower_expr(value_expr)?;
        // The new array owns the appended element.
        let x = self.bind_owned(value_expr, x0, &elem);
        let elem_llvm = self.types.llvm(&elem);
        let elem_sz = self.elem_bytes(&elem);
        let new_len = self.builder.build_int_add(len, self.i64t().const_int(1, false), "nlen").unwrap();
        let buf = self.alloc_array_buf(new_len, elem_sz);
        // Functional push doesn't consume the source, so the new buffer must own
        // independent copies of managed elements (else both would free them).
        if let Some(cf) = self.get_or_build_clone(&elem) {
            let entry = self.builder.get_insert_block().unwrap();
            let head = self.context.append_basic_block(self.llvm_fn, "pcp.head");
            let body = self.context.append_basic_block(self.llvm_fn, "pcp.body");
            let exit = self.context.append_basic_block(self.llvm_fn, "pcp.exit");
            self.builder.build_unconditional_branch(head).unwrap();
            self.builder.position_at_end(head);
            let phi = self.builder.build_phi(self.i64t(), "i").unwrap();
            phi.add_incoming(&[(&self.i64t().const_zero(), entry)]);
            let i = phi.as_basic_value().into_int_value();
            let c = self.builder.build_int_compare(IntPredicate::SLT, i, len, "c").unwrap();
            self.builder.build_conditional_branch(c, body, exit).unwrap();
            self.builder.position_at_end(body);
            let sp = unsafe { self.builder.build_in_bounds_gep(elem_llvm, data, &[i], "sp").unwrap() };
            let ev = self.builder.build_load(elem_llvm, sp, "ev").unwrap();
            let cv = self.builder.build_call(cf, &[ev.into()], "cv").unwrap().try_as_basic_value().basic().unwrap();
            let dp = unsafe { self.builder.build_in_bounds_gep(elem_llvm, buf, &[i], "dp").unwrap() };
            self.builder.build_store(dp, cv).unwrap();
            let nx = self.builder.build_int_add(i, self.i64t().const_int(1, false), "nx").unwrap();
            phi.add_incoming(&[(&nx, self.builder.get_insert_block().unwrap())]);
            self.builder.build_unconditional_branch(head).unwrap();
            self.builder.position_at_end(exit);
        } else {
            let old_bytes = self.builder.build_int_mul(len, elem_sz, "obytes").unwrap();
            self.memcpy_bytes(buf, data, old_bytes);
        }
        let end = unsafe { self.builder.build_in_bounds_gep(elem_llvm, buf, &[len], "endp").unwrap() };
        self.builder.build_store(end, x).unwrap();
        Ok((self.make_len_ptr(new_len, buf).into(), ZType::Array(Box::new(elem))))
    }

    /// `array_repeat(value, count)` → `{n, buf}` of `n = max(count, 0)` independent
    /// copies of `value` (the element ZType is read from `value`). Managed elements
    /// are cloned per slot; the consumed seed is dropped once. Matches the
    /// interpreter's `vec![value; n]`.
    fn lower_array_repeat(
        &mut self,
        value_expr: &MirExpr,
        count_expr: &MirExpr,
    ) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        let (v0, elem) = self.lower_expr(value_expr)?;
        // An independent, owned seed (clone if it aliases a local).
        let seed = self.bind_owned(value_expr, v0, &elem);
        let count = self.lower_int(count_expr)?;
        let zero = self.i64t().const_zero();
        let is_neg = self
            .builder
            .build_int_compare(IntPredicate::SLT, count, zero, "rneg")
            .unwrap();
        let n = self
            .builder
            .build_select(is_neg, zero, count, "rn")
            .unwrap()
            .into_int_value();
        let elem_llvm = self.types.llvm(&elem);
        let elem_sz = self.elem_bytes(&elem);
        let buf = self.alloc_array_buf(n, elem_sz);
        let clone_fn = self.get_or_build_clone(&elem);
        // Loop i in 0..n: store an (independent) copy of the seed at buf[i].
        let entry = self.builder.get_insert_block().unwrap();
        let head = self.context.append_basic_block(self.llvm_fn, "rep.head");
        let body = self.context.append_basic_block(self.llvm_fn, "rep.body");
        let exit = self.context.append_basic_block(self.llvm_fn, "rep.exit");
        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let phi = self.builder.build_phi(self.i64t(), "i").unwrap();
        phi.add_incoming(&[(&zero, entry)]);
        let i = phi.as_basic_value().into_int_value();
        let c = self
            .builder
            .build_int_compare(IntPredicate::SLT, i, n, "c")
            .unwrap();
        self.builder.build_conditional_branch(c, body, exit).unwrap();
        self.builder.position_at_end(body);
        let slot = unsafe {
            self.builder
                .build_in_bounds_gep(elem_llvm, buf, &[i], "rp")
                .unwrap()
        };
        let stored = match clone_fn {
            Some(cf) => self
                .builder
                .build_call(cf, &[seed.into()], "rc")
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap(),
            None => seed,
        };
        self.builder.build_store(slot, stored).unwrap();
        let nx = self
            .builder
            .build_int_add(i, self.i64t().const_int(1, false), "nx")
            .unwrap();
        phi.add_incoming(&[(&nx, self.builder.get_insert_block().unwrap())]);
        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(exit);
        // The seed itself was consumed by the repeat — drop the managed original.
        if let Some(df) = self.get_or_build_drop(&elem) {
            self.builder.build_call(df, &[seed.into()], "rdrop").unwrap();
        }
        Ok((self.make_len_ptr(n, buf).into(), ZType::Array(Box::new(elem))))
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

        // Copy-on-write: a shared buffer must become unique before we mutate it.
        let (len, ptr) = self.cow_make_unique(slot, &elem);
        let cap = self.array_cap(ptr);
        let (v0, _) = self.lower_expr(value_arg)?;
        // The array owns the appended element (existing elements stay owned by
        // this same buffer; grow MOVES them — memcpy then frees only the buffer).
        let v = self.bind_owned(value_arg, v0, &elem);

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
        // Free the outgrown buffer: this local uniquely owns it (value
        // semantics), and after the copy + slot update nothing references it.
        self.free_array_data(ptr);
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
    /// The compile-time CSR number for a `csr_*` builtin — required to be an
    /// integer literal, since the CSR is encoded directly in the instruction.
    fn const_csr(&self, expr: &MirExpr) -> Result<i64, String> {
        match expr {
            MirExpr::Int(text) => text
                .parse()
                .map_err(|_| format!("bad CSR literal `{text}`")),
            _ => Err("csr_* requires an integer-literal CSR number".into()),
        }
    }

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
        let (rv, rt) = self.lower_expr(right)?;
        // Operator overloading: a non-scalar (struct/enum) left operand dispatches
        // `a OP b` to its operator trait method `{name}${Base}` (e.g. `+` →
        // `add$Point`), with both operands taken by value (bind_owned). Scalars
        // fall through to the built-in paths below.
        if matches!(lt, ZType::Struct(_) | ZType::Enum(_)) {
            if let Some(method) = crate::mir::operator_trait_method(op) {
                let dispatch = crate::type_syntax::dispatch_name(method, &zty_base_name(&lt));
                if let Some(&func) = self.functions.get(&dispatch) {
                    let lo = self.bind_owned(left, lv, &lt);
                    let ro = self.bind_owned(right, rv, &rt);
                    let call = self
                        .builder
                        .build_call(func, &[lo.into(), ro.into()], "opcall")
                        .unwrap();
                    let val = call
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| format!("`{dispatch}` returned no value"))?;
                    let ret = self.types.returns.get(&dispatch).cloned().unwrap_or(lt.clone());
                    return Ok((val, ret));
                }
            }
        }
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

    /// Byte index of the first occurrence of `sub` in `s`, or -1 (empty `sub` → 0).
    /// Mirrors `runtime::byte_index_of` exactly so native == interpreter.
    /// `int_pow(base, exp)`: exp<0 ⇒ 0, else product of `base` `exp` times
    /// (result starts at 1; i64 multiply wraps). Mirrors the interpreter loop.
    fn gen_int_pow(&self, base: IntValue<'ctx>, exp: IntValue<'ctx>) -> IntValue<'ctx> {
        let b = self.builder;
        let i64t = self.i64t();
        let res = self.entry_alloca("powres", i64t.into());
        let i = self.entry_alloca("powi", i64t.into());
        let is_neg = b
            .build_int_compare(IntPredicate::SLT, exp, i64t.const_zero(), "expneg")
            .unwrap();
        let neg = self.context.append_basic_block(self.llvm_fn, "pow.neg");
        let init = self.context.append_basic_block(self.llvm_fn, "pow.init");
        let head = self.context.append_basic_block(self.llvm_fn, "pow.head");
        let body = self.context.append_basic_block(self.llvm_fn, "pow.body");
        let done = self.context.append_basic_block(self.llvm_fn, "pow.done");

        b.build_conditional_branch(is_neg, neg, init).unwrap();
        b.position_at_end(neg);
        b.build_store(res, i64t.const_zero()).unwrap();
        b.build_unconditional_branch(done).unwrap();

        b.position_at_end(init);
        b.build_store(res, i64t.const_int(1, false)).unwrap();
        b.build_store(i, i64t.const_zero()).unwrap();
        b.build_unconditional_branch(head).unwrap();

        b.position_at_end(head);
        let iv = b.build_load(i64t, i, "i").unwrap().into_int_value();
        let cond = b
            .build_int_compare(IntPredicate::SLT, iv, exp, "iltexp")
            .unwrap();
        b.build_conditional_branch(cond, body, done).unwrap();

        b.position_at_end(body);
        let r = b.build_load(i64t, res, "r").unwrap().into_int_value();
        let r2 = b.build_int_mul(r, base, "rmul").unwrap();
        b.build_store(res, r2).unwrap();
        let i2 = b
            .build_int_add(iv, i64t.const_int(1, false), "i2")
            .unwrap();
        b.build_store(i, i2).unwrap();
        b.build_unconditional_branch(head).unwrap();

        b.position_at_end(done);
        b.build_load(i64t, res, "powval").unwrap().into_int_value()
    }

    /// `string_to_int(s)`: optional leading `-` then ASCII digits; any non-digit
    /// (or empty / lone `-`) ⇒ 0. i64 arithmetic wraps. Byte-for-byte equivalent
    /// to the interpreter's `parse_decimal_i64`.
    fn gen_string_to_int(&self, s: inkwell::values::StructValue<'ctx>) -> IntValue<'ctx> {
        let b = self.builder;
        let i64t = self.i64t();
        let i8t = self.context.i8_type();
        let (len, data) = self.len_ptr_parts(s);
        let res = self.entry_alloca("s2ires", i64t.into());
        let sign = self.entry_alloca("s2isign", i64t.into());
        let valid = self.entry_alloca("s2ivalid", i64t.into());
        let start = self.entry_alloca("s2istart", i64t.into());
        let idx = self.entry_alloca("s2ii", i64t.into());
        b.build_store(res, i64t.const_zero()).unwrap();
        b.build_store(sign, i64t.const_int(1, false)).unwrap();
        b.build_store(valid, i64t.const_int(1, false)).unwrap();
        b.build_store(start, i64t.const_zero()).unwrap();

        let chk = self.context.append_basic_block(self.llvm_fn, "s2i.chkminus");
        let setminus = self.context.append_basic_block(self.llvm_fn, "s2i.setminus");
        let afterminus = self.context.append_basic_block(self.llvm_fn, "s2i.afterminus");
        let emptyblk = self.context.append_basic_block(self.llvm_fn, "s2i.empty");
        let loopinit = self.context.append_basic_block(self.llvm_fn, "s2i.init");
        let head = self.context.append_basic_block(self.llvm_fn, "s2i.head");
        let body = self.context.append_basic_block(self.llvm_fn, "s2i.body");
        let done = self.context.append_basic_block(self.llvm_fn, "s2i.done");

        // First byte `-` ⇒ sign=-1, start=1 (guarded by len>0).
        let has_len = b
            .build_int_compare(IntPredicate::SGT, len, i64t.const_zero(), "haslen")
            .unwrap();
        b.build_conditional_branch(has_len, chk, afterminus).unwrap();
        b.position_at_end(chk);
        let c0 = b
            .build_load(i8t, data, "c0")
            .unwrap()
            .into_int_value();
        let c0w = b.build_int_z_extend(c0, i64t, "c0w").unwrap();
        let is_minus = b
            .build_int_compare(IntPredicate::EQ, c0w, i64t.const_int(45, false), "isminus")
            .unwrap();
        b.build_conditional_branch(is_minus, setminus, afterminus).unwrap();
        b.position_at_end(setminus);
        b.build_store(start, i64t.const_int(1, false)).unwrap();
        b.build_store(sign, i64t.const_all_ones()).unwrap(); // -1
        b.build_unconditional_branch(afterminus).unwrap();

        b.position_at_end(afterminus);
        let startv = b.build_load(i64t, start, "startv").unwrap().into_int_value();
        // start >= len ⇒ empty / lone `-` ⇒ result 0.
        let empty = b
            .build_int_compare(IntPredicate::SGE, startv, len, "empty")
            .unwrap();
        b.build_conditional_branch(empty, emptyblk, loopinit).unwrap();
        b.position_at_end(emptyblk);
        b.build_store(valid, i64t.const_zero()).unwrap();
        b.build_unconditional_branch(done).unwrap();

        b.position_at_end(loopinit);
        b.build_store(idx, startv).unwrap();
        b.build_unconditional_branch(head).unwrap();

        b.position_at_end(head);
        let iv = b.build_load(i64t, idx, "iv").unwrap().into_int_value();
        let cond = b
            .build_int_compare(IntPredicate::SLT, iv, len, "iltlen")
            .unwrap();
        b.build_conditional_branch(cond, body, done).unwrap();

        b.position_at_end(body);
        let ptr = unsafe { b.build_in_bounds_gep(i8t, data, &[iv], "cptr").unwrap() };
        let cb = b.build_load(i8t, ptr, "cb").unwrap().into_int_value();
        let ci = b.build_int_z_extend(cb, i64t, "ci").unwrap();
        let ge0 = b
            .build_int_compare(IntPredicate::SGE, ci, i64t.const_int(48, false), "ge0")
            .unwrap();
        let le9 = b
            .build_int_compare(IntPredicate::SLE, ci, i64t.const_int(57, false), "le9")
            .unwrap();
        let isdigit = b.build_and(ge0, le9, "isdigit").unwrap();
        let isdig_i64 = b.build_int_z_extend(isdigit, i64t, "isdigw").unwrap();
        let v = b.build_load(i64t, valid, "v").unwrap().into_int_value();
        let v2 = b.build_and(v, isdig_i64, "v2").unwrap();
        b.build_store(valid, v2).unwrap();
        let r = b.build_load(i64t, res, "ra").unwrap().into_int_value();
        let r10 = b
            .build_int_mul(r, i64t.const_int(10, false), "r10")
            .unwrap();
        let digit = b
            .build_int_sub(ci, i64t.const_int(48, false), "digit")
            .unwrap();
        let r2 = b.build_int_add(r10, digit, "racc").unwrap();
        b.build_store(res, r2).unwrap();
        let i2 = b.build_int_add(iv, i64t.const_int(1, false), "i2").unwrap();
        b.build_store(idx, i2).unwrap();
        b.build_unconditional_branch(head).unwrap();

        b.position_at_end(done);
        let v = b.build_load(i64t, valid, "vf").unwrap().into_int_value();
        let resf = b.build_load(i64t, res, "resf").unwrap().into_int_value();
        let signf = b.build_load(i64t, sign, "signf").unwrap().into_int_value();
        let signed = b.build_int_mul(signf, resf, "signed").unwrap();
        let isvalid = b
            .build_int_compare(IntPredicate::NE, v, i64t.const_zero(), "isvalid")
            .unwrap();
        b.build_select(isvalid, signed, i64t.const_zero(), "s2ival")
            .unwrap()
            .into_int_value()
    }

    /// Self-contained signed-i64 → decimal string, replacing libc `snprintf`
    /// (so the native backend — and the freestanding kernel — needs no libc).
    /// Works on the UNSIGNED magnitude so `i64::MIN` is handled without overflow
    /// (`0 - MIN` wraps back to MIN, whose unsigned value is the right
    /// magnitude). Two passes: count digits, then fill them in backward.
    fn gen_int_to_string(&self, n: IntValue<'ctx>) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let i64t = self.i64t();
        let i8t = self.context.i8_type();
        let zero = i64t.const_zero();
        let ten = i64t.const_int(10, false);

        let isneg = b.build_int_compare(IntPredicate::SLT, n, zero, "itsneg").unwrap();
        let negbit = b.build_int_z_extend(isneg, i64t, "itnegbit").unwrap();
        let negval = b.build_int_sub(zero, n, "itnegval").unwrap();
        let mag0 = b.build_select(isneg, negval, n, "itmag0").unwrap().into_int_value();

        let mag = self.entry_alloca("itmag", i64t.into());
        let cnt = self.entry_alloca("itcnt", i64t.into());
        let pos = self.entry_alloca("itpos", i64t.into());
        let idx = self.entry_alloca("itidx", i64t.into());

        // Pass 1: count digits (do-while ⇒ at least one, so 0 → "0").
        b.build_store(mag, mag0).unwrap();
        b.build_store(cnt, zero).unwrap();
        let chead = self.context.append_basic_block(self.llvm_fn, "it.chead");
        let cdone = self.context.append_basic_block(self.llvm_fn, "it.cdone");
        b.build_unconditional_branch(chead).unwrap();
        b.position_at_end(chead);
        let c = b.build_load(i64t, cnt, "c").unwrap().into_int_value();
        b.build_store(cnt, b.build_int_add(c, i64t.const_int(1, false), "c1").unwrap()).unwrap();
        let m = b.build_load(i64t, mag, "m").unwrap().into_int_value();
        let m2 = b.build_int_unsigned_div(m, ten, "mdiv").unwrap();
        b.build_store(mag, m2).unwrap();
        let more = b.build_int_compare(IntPredicate::NE, m2, zero, "more").unwrap();
        b.build_conditional_branch(more, chead, cdone).unwrap();

        b.position_at_end(cdone);
        let digits = b.build_load(i64t, cnt, "digits").unwrap().into_int_value();
        let total = b.build_int_add(digits, negbit, "ittotal").unwrap();
        let buf = self.alloc_str_buf(total);
        // Leading '-' for negatives (digits then fill positions [negbit, total-1]).
        let minus_bb = self.context.append_basic_block(self.llvm_fn, "it.minus");
        let fillinit = self.context.append_basic_block(self.llvm_fn, "it.fillinit");
        b.build_conditional_branch(isneg, minus_bb, fillinit).unwrap();
        b.position_at_end(minus_bb);
        b.build_store(buf, i8t.const_int(45, false)).unwrap(); // '-'
        b.build_unconditional_branch(fillinit).unwrap();

        // Pass 2: write digits backward from the end of the buffer.
        b.position_at_end(fillinit);
        b.build_store(mag, mag0).unwrap();
        b.build_store(idx, zero).unwrap();
        b.build_store(pos, b.build_int_sub(total, i64t.const_int(1, false), "lastpos").unwrap()).unwrap();
        let fhead = self.context.append_basic_block(self.llvm_fn, "it.fhead");
        let fbody = self.context.append_basic_block(self.llvm_fn, "it.fbody");
        let fdone = self.context.append_basic_block(self.llvm_fn, "it.fdone");
        b.build_unconditional_branch(fhead).unwrap();
        b.position_at_end(fhead);
        let iv = b.build_load(i64t, idx, "iv").unwrap().into_int_value();
        let cond = b.build_int_compare(IntPredicate::SLT, iv, digits, "fcond").unwrap();
        b.build_conditional_branch(cond, fbody, fdone).unwrap();
        b.position_at_end(fbody);
        let m = b.build_load(i64t, mag, "fm").unwrap().into_int_value();
        let d = b.build_int_unsigned_rem(m, ten, "frem").unwrap();
        let d8 = b.build_int_truncate(d, i8t, "fd8").unwrap();
        let ch = b.build_int_add(d8, i8t.const_int(48, false), "fch").unwrap();
        let p = b.build_load(i64t, pos, "fp").unwrap().into_int_value();
        let dst = unsafe { b.build_in_bounds_gep(i8t, buf, &[p], "fdst").unwrap() };
        b.build_store(dst, ch).unwrap();
        b.build_store(mag, b.build_int_unsigned_div(m, ten, "fmdiv").unwrap()).unwrap();
        b.build_store(pos, b.build_int_sub(p, i64t.const_int(1, false), "p1").unwrap()).unwrap();
        b.build_store(idx, b.build_int_add(iv, i64t.const_int(1, false), "iv1").unwrap()).unwrap();
        b.build_unconditional_branch(fhead).unwrap();

        b.position_at_end(fdone);
        self.make_len_ptr(total, buf)
    }

    fn gen_index_of(
        &self,
        s: inkwell::values::StructValue<'ctx>,
        sub: inkwell::values::StructValue<'ctx>,
    ) -> IntValue<'ctx> {
        let i64t = self.i64t();
        let (slen, sdata) = self.len_ptr_parts(s);
        let (sublen, subdata) = self.len_ptr_parts(sub);
        let res = self.entry_alloca("ixres", i64t.into());

        let is_empty = self.builder.build_int_compare(IntPredicate::EQ, sublen, i64t.const_zero(), "subempty").unwrap();
        let empty_blk = self.context.append_basic_block(self.llvm_fn, "io.empty");
        let chk_len = self.context.append_basic_block(self.llvm_fn, "io.chklen");
        let too_long = self.context.append_basic_block(self.llvm_fn, "io.toolong");
        let setup = self.context.append_basic_block(self.llvm_fn, "io.setup");
        let head = self.context.append_basic_block(self.llvm_fn, "io.head");
        let body = self.context.append_basic_block(self.llvm_fn, "io.body");
        let next = self.context.append_basic_block(self.llvm_fn, "io.next");
        let found = self.context.append_basic_block(self.llvm_fn, "io.found");
        let done = self.context.append_basic_block(self.llvm_fn, "io.done");

        self.builder.build_conditional_branch(is_empty, empty_blk, chk_len).unwrap();
        self.builder.position_at_end(empty_blk);
        self.builder.build_store(res, i64t.const_zero()).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();

        self.builder.position_at_end(chk_len);
        let longer = self.builder.build_int_compare(IntPredicate::SGT, sublen, slen, "sublong").unwrap();
        self.builder.build_conditional_branch(longer, too_long, setup).unwrap();
        self.builder.position_at_end(too_long);
        self.builder.build_store(res, i64t.const_all_ones()).unwrap(); // -1
        self.builder.build_unconditional_branch(done).unwrap();

        self.builder.position_at_end(setup);
        let last = self.builder.build_int_sub(slen, sublen, "last").unwrap();
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(head);
        let phi = self.builder.build_phi(i64t, "i").unwrap();
        phi.add_incoming(&[(&i64t.const_zero(), setup)]);
        let i = phi.as_basic_value().into_int_value();
        let in_range = self.builder.build_int_compare(IntPredicate::SLE, i, last, "inrange").unwrap();
        // Exhausted (i > last) → reuse the `too_long` block, which stores -1.
        self.builder.build_conditional_branch(in_range, body, too_long).unwrap();

        self.builder.position_at_end(body);
        let i8t = self.context.i8_type();
        let ptr = unsafe { self.builder.build_in_bounds_gep(i8t, sdata, &[i], "sptr").unwrap() };
        let cmp = self
            .builder
            .build_call(self.memcmp, &[ptr.into(), subdata.into(), sublen.into()], "mc")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let eq = self.builder.build_int_compare(IntPredicate::EQ, cmp, self.context.i32_type().const_zero(), "eq0").unwrap();
        self.builder.build_conditional_branch(eq, found, next).unwrap();

        self.builder.position_at_end(found);
        self.builder.build_store(res, i).unwrap();
        self.builder.build_unconditional_branch(done).unwrap();

        self.builder.position_at_end(next);
        let i2 = self.builder.build_int_add(i, i64t.const_int(1, false), "i2").unwrap();
        phi.add_incoming(&[(&i2, next)]);
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(done);
        self.builder.build_load(i64t, res, "ixval").unwrap().into_int_value()
    }

    /// `s` repeated `n` times (n<0 → empty) as a fresh heap string.
    fn gen_repeat(
        &self,
        s: inkwell::values::StructValue<'ctx>,
        n: IntValue<'ctx>,
    ) -> inkwell::values::StructValue<'ctx> {
        let i64t = self.i64t();
        let (slen, sdata) = self.len_ptr_parts(s);
        let neg = self.builder.build_int_compare(IntPredicate::SLT, n, i64t.const_zero(), "neg").unwrap();
        let count = self.builder.build_select(neg, i64t.const_zero(), n, "count").unwrap().into_int_value();
        let total = self.builder.build_int_mul(slen, count, "rtot").unwrap();
        let buf = self.alloc_str_buf(total);

        let head = self.context.append_basic_block(self.llvm_fn, "rp.head");
        let body = self.context.append_basic_block(self.llvm_fn, "rp.body");
        let exit = self.context.append_basic_block(self.llvm_fn, "rp.exit");
        let entry = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let phi = self.builder.build_phi(i64t, "k").unwrap();
        phi.add_incoming(&[(&i64t.const_zero(), entry)]);
        let k = phi.as_basic_value().into_int_value();
        let cond = self.builder.build_int_compare(IntPredicate::SLT, k, count, "rcond").unwrap();
        self.builder.build_conditional_branch(cond, body, exit).unwrap();
        self.builder.position_at_end(body);
        let off = self.builder.build_int_mul(k, slen, "roff").unwrap();
        let i8t = self.context.i8_type();
        let dst = unsafe { self.builder.build_in_bounds_gep(i8t, buf, &[off], "rdst").unwrap() };
        self.memcpy_bytes(dst, sdata, slen);
        let k2 = self.builder.build_int_add(k, i64t.const_int(1, false), "k2").unwrap();
        phi.add_incoming(&[(&k2, self.builder.get_insert_block().unwrap())]);
        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(exit);
        self.make_len_ptr(total, buf)
    }

    /// `i1` true when the i64 byte value `ci` is ASCII whitespace, matching
    /// Rust's `is_ascii_whitespace`: space, \t, \n, \r, form-feed (NOT \x0B).
    fn is_ws_byte(&self, ci: IntValue<'ctx>) -> IntValue<'ctx> {
        let b = self.builder;
        let i64t = self.i64t();
        let mut acc: Option<IntValue<'ctx>> = None;
        for code in [9u64, 10, 12, 13, 32] {
            let eq = b
                .build_int_compare(IntPredicate::EQ, ci, i64t.const_int(code, false), "wseq")
                .unwrap();
            acc = Some(match acc {
                None => eq,
                Some(prev) => b.build_or(prev, eq, "wsor").unwrap(),
            });
        }
        acc.unwrap()
    }

    /// ASCII case map: a fresh `malloc(len)` string with each byte upper/lower-
    /// cased in place (`to_lower` picks the direction). Mirrors the interpreter's
    /// `to_ascii_uppercase`/`to_ascii_lowercase` — only A–Z / a–z bytes shift by
    /// 32, so UTF-8 continuation bytes (≥ 0x80) pass through untouched.
    fn gen_case_map(
        &self,
        s: inkwell::values::StructValue<'ctx>,
        to_lower: bool,
    ) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let i64t = self.i64t();
        let i8t = self.context.i8_type();
        let (len, data) = self.len_ptr_parts(s);
        let buf = self.alloc_str_buf(len);
        // Lowercase maps A–Z (65–90) by +32; uppercase maps a–z (97–122) by −32.
        let (lo, hi) = if to_lower { (65u64, 90u64) } else { (97u64, 122u64) };

        let head = self.context.append_basic_block(self.llvm_fn, "cm.head");
        let body = self.context.append_basic_block(self.llvm_fn, "cm.body");
        let exit = self.context.append_basic_block(self.llvm_fn, "cm.exit");
        let entry = b.get_insert_block().unwrap();
        b.build_unconditional_branch(head).unwrap();
        b.position_at_end(head);
        let phi = b.build_phi(i64t, "i").unwrap();
        phi.add_incoming(&[(&i64t.const_zero(), entry)]);
        let i = phi.as_basic_value().into_int_value();
        let cond = b.build_int_compare(IntPredicate::SLT, i, len, "cmcond").unwrap();
        b.build_conditional_branch(cond, body, exit).unwrap();

        b.position_at_end(body);
        let src = unsafe { b.build_in_bounds_gep(i8t, data, &[i], "cmsrc").unwrap() };
        let cb = b.build_load(i8t, src, "cmcb").unwrap().into_int_value();
        let ci = b.build_int_z_extend(cb, i64t, "cmci").unwrap();
        let ge = b.build_int_compare(IntPredicate::SGE, ci, i64t.const_int(lo, false), "cmge").unwrap();
        let le = b.build_int_compare(IntPredicate::SLE, ci, i64t.const_int(hi, false), "cmle").unwrap();
        let in_range = b.build_and(ge, le, "cmin").unwrap();
        let delta = i8t.const_int(32, false);
        let shifted = if to_lower {
            b.build_int_add(cb, delta, "cmadd").unwrap()
        } else {
            b.build_int_sub(cb, delta, "cmsub").unwrap()
        };
        let newb = b.build_select(in_range, shifted, cb, "cmnew").unwrap().into_int_value();
        let dst = unsafe { b.build_in_bounds_gep(i8t, buf, &[i], "cmdst").unwrap() };
        b.build_store(dst, newb).unwrap();
        let i2 = b.build_int_add(i, i64t.const_int(1, false), "cmi2").unwrap();
        phi.add_incoming(&[(&i2, b.get_insert_block().unwrap())]);
        b.build_unconditional_branch(head).unwrap();

        b.position_at_end(exit);
        self.make_len_ptr(len, buf)
    }

    /// Trim leading/trailing ASCII whitespace: scan a `[start, end)` byte range
    /// (mirrors `byte_trim_range`), then `malloc(end-start)` + memcpy. An all-
    /// whitespace input yields an empty string.
    fn gen_trim(
        &self,
        s: inkwell::values::StructValue<'ctx>,
    ) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let i64t = self.i64t();
        let i8t = self.context.i8_type();
        let (len, data) = self.len_ptr_parts(s);
        let start = self.entry_alloca("trstart", i64t.into());
        let end = self.entry_alloca("trend", i64t.into());
        b.build_store(start, i64t.const_zero()).unwrap();
        b.build_store(end, len).unwrap();

        // Advance `start` while start < end-equivalent (start < len) and ws.
        let shead = self.context.append_basic_block(self.llvm_fn, "tr.shead");
        let sbody = self.context.append_basic_block(self.llvm_fn, "tr.sbody");
        let safter = self.context.append_basic_block(self.llvm_fn, "tr.safter");
        b.build_unconditional_branch(shead).unwrap();
        b.position_at_end(shead);
        let sv = b.build_load(i64t, start, "sv").unwrap().into_int_value();
        let in_bounds = b.build_int_compare(IntPredicate::SLT, sv, len, "sib").unwrap();
        b.build_conditional_branch(in_bounds, sbody, safter).unwrap();
        b.position_at_end(sbody);
        let sptr = unsafe { b.build_in_bounds_gep(i8t, data, &[sv], "sptr").unwrap() };
        let scb = b.build_load(i8t, sptr, "scb").unwrap().into_int_value();
        let sci = b.build_int_z_extend(scb, i64t, "sci").unwrap();
        let sws = self.is_ws_byte(sci);
        let snext = self.context.append_basic_block(self.llvm_fn, "tr.snext");
        b.build_conditional_branch(sws, snext, safter).unwrap();
        b.position_at_end(snext);
        let sv2 = b.build_int_add(sv, i64t.const_int(1, false), "sv2").unwrap();
        b.build_store(start, sv2).unwrap();
        b.build_unconditional_branch(shead).unwrap();

        // Retreat `end` while end > start and bytes[end-1] is ws.
        b.position_at_end(safter);
        let ehead = self.context.append_basic_block(self.llvm_fn, "tr.ehead");
        let ebody = self.context.append_basic_block(self.llvm_fn, "tr.ebody");
        let eafter = self.context.append_basic_block(self.llvm_fn, "tr.eafter");
        b.build_unconditional_branch(ehead).unwrap();
        b.position_at_end(ehead);
        let ev = b.build_load(i64t, end, "ev").unwrap().into_int_value();
        let svf = b.build_load(i64t, start, "svf").unwrap().into_int_value();
        let gt = b.build_int_compare(IntPredicate::SGT, ev, svf, "egt").unwrap();
        b.build_conditional_branch(gt, ebody, eafter).unwrap();
        b.position_at_end(ebody);
        let em1 = b.build_int_sub(ev, i64t.const_int(1, false), "em1").unwrap();
        let eptr = unsafe { b.build_in_bounds_gep(i8t, data, &[em1], "eptr").unwrap() };
        let ecb = b.build_load(i8t, eptr, "ecb").unwrap().into_int_value();
        let eci = b.build_int_z_extend(ecb, i64t, "eci").unwrap();
        let ews = self.is_ws_byte(eci);
        let enext = self.context.append_basic_block(self.llvm_fn, "tr.enext");
        b.build_conditional_branch(ews, enext, eafter).unwrap();
        b.position_at_end(enext);
        b.build_store(end, em1).unwrap();
        b.build_unconditional_branch(ehead).unwrap();

        b.position_at_end(eafter);
        let sfin = b.build_load(i64t, start, "sfin").unwrap().into_int_value();
        let efin = b.build_load(i64t, end, "efin").unwrap().into_int_value();
        let newlen = b.build_int_sub(efin, sfin, "trlen").unwrap();
        let buf = self.alloc_str_buf(newlen);
        let src = unsafe { b.build_in_bounds_gep(i8t, data, &[sfin], "trsrc").unwrap() };
        self.memcpy_bytes(buf, src, newlen);
        self.make_len_ptr(newlen, buf)
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
