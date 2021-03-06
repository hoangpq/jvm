#![allow(non_snake_case)]

use crate::classfile::{self, access_flags as acc, constant_pool, consts as cls_file_const};
use crate::native::{common, new_fn, JNIEnv, JNINativeMethod, JNIResult};
use crate::oop::{self, ClassKind, Oop, ValueType};
use crate::runtime::{self, require_class2, require_class3, JavaThread};
use crate::types::{ClassRef, MethodIdRef};
use crate::util;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub fn get_primitive_class_mirror(key: &str) -> Option<Oop> {
    //todo: avoid mutex lock, it's only read
    let mirrors = PRIM_MIRROS.read().unwrap();
    mirrors.get(key).map(|it| it.clone())
}

pub fn get_native_methods() -> Vec<JNINativeMethod> {
    vec![
        new_fn("registerNatives", "()V", Box::new(jvm_registerNatives)),
        new_fn(
            "desiredAssertionStatus0",
            "(Ljava/lang/Class;)Z",
            Box::new(jvm_desiredAssertionStatus0),
        ),
        new_fn(
            "getPrimitiveClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            Box::new(jvm_getPrimitiveClass),
        ),
        new_fn(
            "getDeclaredFields0",
            "(Z)[Ljava/lang/reflect/Field;",
            Box::new(jvm_getDeclaredFields0),
        ),
        new_fn("getName0", "()Ljava/lang/String;", Box::new(jvm_getName0)),
        new_fn(
            "forName0",
            "(Ljava/lang/String;ZLjava/lang/ClassLoader;Ljava/lang/Class;)Ljava/lang/Class;",
            Box::new(jvm_forName0),
        ),
        new_fn("isPrimitive", "()Z", Box::new(jvm_isPrimitive)),
        new_fn(
            "isAssignableFrom",
            "(Ljava/lang/Class;)Z",
            Box::new(jvm_isAssignableFrom),
        ),
        new_fn("isInterface", "()Z", Box::new(jvm_isInterface)),
        new_fn(
            "getDeclaredConstructors0",
            "(Z)[Ljava/lang/reflect/Constructor;",
            Box::new(jvm_getDeclaredConstructors0),
        ),
        new_fn("getModifiers", "()I", Box::new(jvm_getModifiers)),
        new_fn(
            "getSuperclass",
            "()Ljava/lang/Class;",
            Box::new(jvm_getSuperclass),
        ),
        new_fn("isArray", "()Z", Box::new(jvm_isArray)),
        new_fn(
            "getComponentType",
            "()Ljava/lang/Class;",
            Box::new(jvm_getComponentType),
        ),
        new_fn(
            "getEnclosingMethod0",
            "()[Ljava/lang/Object;",
            Box::new(jvm_getEnclosingMethod0),
        ),
        new_fn(
            "getDeclaringClass0",
            "()Ljava/lang/Class;",
            Box::new(jvm_getDeclaringClass0),
        ),
        new_fn(
            "isInstance",
            "(Ljava/lang/Object;)Z",
            Box::new(jvm_isInstance),
        ),
        new_fn(
            "getDeclaredMethods0",
            "(Z)[Ljava/lang/reflect/Method;",
            Box::new(jvm_getDeclaredMethods0),
        ),
        new_fn(
            "getInterfaces0",
            "()[Ljava/lang/Class;",
            Box::new(jvm_getInterfaces0),
        ),
        new_fn("getRawAnnotations", "()[B", Box::new(jvm_getRawAnnotations)),
        new_fn(
            "getConstantPool",
            "()Lsun/reflect/ConstantPool;",
            Box::new(jvm_getConstantPool),
        ),
        new_fn(
            "getDeclaredClasses0",
            "()[Ljava/lang/Class;",
            Box::new(jvm_getDeclaredClasses0),
        ),
    ]
}

#[derive(Copy, Clone, PartialEq)]
enum ClassMirrorState {
    NotFixed,
    Fixed,
}

lazy_static! {
    static ref MIRROR_STATE: RwLock<ClassMirrorState> = { RwLock::new(ClassMirrorState::NotFixed) };
    static ref PRIM_MIRROS: RwLock<HashMap<String, Oop>> = {
        let hm = HashMap::new();
        RwLock::new(hm)
    };
    static ref SIGNATURE_DIC: HashMap<&'static str, &'static str> = {
        let dic: HashMap<&'static str, &'static str> = [
            ("byte", "B"),
            ("boolean", "Z"),
            ("char", "C"),
            ("short", "S"),
            ("int", "I"),
            ("float", "F"),
            ("long", "J"),
            ("double", "D"),
            ("void", "V"),
        ]
        .iter()
        .cloned()
        .collect();

        dic
    };
    static ref DELAYED_MIRROS: RwLock<Vec<String>> = {
        let v = vec![
            "I", "Z", "B", "C", "S", "F", "J", "D", "V", "[I", "[Z", "[B", "[C", "[S", "[F", "[J",
            "[D",
        ];
        let v: Vec<String> = v.iter().map(|it| it.to_string()).collect();
        RwLock::new(v)
    };
    static ref DELAYED_ARY_MIRROS: RwLock<Vec<ClassRef>> = {
        let v = vec![];
        RwLock::new(v)
    };
}

pub fn init() {
    lazy_static::initialize(&MIRROR_STATE);
    lazy_static::initialize(&SIGNATURE_DIC);
    lazy_static::initialize(&PRIM_MIRROS);
    lazy_static::initialize(&DELAYED_MIRROS);
    lazy_static::initialize(&DELAYED_ARY_MIRROS);
}

pub fn create_mirror(cls: ClassRef) {
    let is_fixed = {
        let s = MIRROR_STATE.write().unwrap();
        *s == ClassMirrorState::Fixed
    };

    if is_fixed {
        let mirror = Oop::new_mirror(cls.clone());
        let mut cls = cls.write().unwrap();
        trace!("mirror created: {}", unsafe {
            std::str::from_utf8_unchecked(cls.name.as_slice())
        });
        cls.set_mirror(mirror);
    } else {
        let cls_back = cls.clone();
        let cls = cls.read().unwrap();
        let name = String::from_utf8_lossy(cls.name.as_slice()).to_string();
        warn!("mirror create delayed: {}", name);
        match cls.kind {
            oop::class::ClassKind::Instance(_) => {
                let mut mirrors = DELAYED_MIRROS.write().unwrap();
                mirrors.push(name);
            }
            _ => {
                let mut mirrors = DELAYED_ARY_MIRROS.write().unwrap();
                mirrors.push(cls_back);
            }
        }
    }
}

/*
called after 'java/lang/Class' inited in init_vm.rs
*/
pub fn create_delayed_mirrors() {
    let names: Vec<String> = {
        let mirros = DELAYED_MIRROS.read().unwrap();
        mirros.clone()
    };

    {
        let mut s = MIRROR_STATE.write().unwrap();
        *s = ClassMirrorState::Fixed;
    }

    for name in names {
        if name.len() > 2 {
            //java.lang.XXX
            let target = require_class3(None, name.as_bytes()).unwrap();
            create_mirror(target);
        } else {
            let is_prim_ary = name.as_bytes()[0] == b'[';
            let (vt, target) = if is_prim_ary {
                let vt = ValueType::from(&name.as_bytes()[1]);
                let target = require_class3(None, name.as_bytes()).unwrap();
                (vt, Some(target))
            } else {
                (ValueType::from(&name.as_bytes()[0]), None)
            };

            let mirror = Oop::new_prim_mirror(vt, target.clone());
            if is_prim_ary {
                let target = target.unwrap();
                let mut cls = target.write().unwrap();
                //                warn!("set_mirror name={}", String::from_utf8_lossy(cls.name.as_slice()));
                cls.set_mirror(mirror.clone());
            }

            let mut mirrors = PRIM_MIRROS.write().unwrap();
            mirrors.insert(name.to_string(), mirror);
        }
    }
}

/*
called after 'java/lang/Class' inited in init_vm.rs
*/
pub fn create_delayed_ary_mirrors() {
    let classes: Vec<ClassRef> = {
        let mirros = DELAYED_ARY_MIRROS.read().unwrap();
        mirros.clone()
    };

    for cls in classes {
        let value_type = {
            let cls = cls.read().unwrap();
            match &cls.kind {
                oop::class::ClassKind::ObjectArray(obj_ary) => obj_ary.value_type,
                oop::class::ClassKind::TypeArray(typ_ary) => typ_ary.value_type,
                _ => unreachable!(),
            }
        };
        let mirror = Oop::new_ary_mirror(cls.clone(), value_type);
        let mut cls = cls.write().unwrap();
        cls.set_mirror(mirror);
    }
}

fn jvm_registerNatives(_jt: &mut JavaThread, _env: JNIEnv, _args: Vec<Oop>) -> JNIResult {
    Ok(None)
}

fn jvm_desiredAssertionStatus0(_jt: &mut JavaThread, _env: JNIEnv, _args: Vec<Oop>) -> JNIResult {
    Ok(Some(Oop::new_int(0)))
}

fn jvm_getPrimitiveClass(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let v = args.get(0).unwrap();
    let v = util::oop::extract_str(v);
    match SIGNATURE_DIC.get(v.as_str()) {
        Some(&s) => Ok(get_primitive_class_mirror(s)),
        _ => unreachable!("Unknown primitive type: {}", v),
    }
}

fn jvm_getDeclaredFields0(jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    //parse args
    let mirror_target = {
        let arg0 = args.get(0).unwrap();
        let arg0 = util::oop::extract_ref(arg0);
        let arg0 = arg0.read().unwrap();
        match &arg0.v {
            oop::RefKind::Mirror(mirror) => mirror.target.clone().unwrap(),
            _ => unreachable!(),
        }
    };

    let public_only = {
        let arg1 = args.get(1).unwrap();
        util::oop::extract_int(arg1) == 1
    };

    //fixme: super fields
    //obtain inst&static fields
    let (inst_fields, static_fields) = {
        let cls = mirror_target.read().unwrap();
        match &cls.kind {
            oop::class::ClassKind::Instance(inst) => {
                (inst.inst_fields.clone(), inst.static_fields.clone())
            }
            _ => unreachable!(),
        }
    };

    //build fields ary
    let mut fields = Vec::new();
    for (_, it) in inst_fields {
        if public_only && !it.field.is_public() {
            continue;
        }

        let v = common::reflect::new_field(jt, it);
        fields.push(v);
    }

    for (_, it) in static_fields {
        if public_only && !it.field.is_public() {
            continue;
        }

        let v = common::reflect::new_field(jt, it);
        fields.push(v);
    }

    //build oop field ar
    let ary_cls = require_class3(None, b"[Ljava/lang/reflect/Field;").unwrap();
    Ok(Some(Oop::new_ref_ary2(ary_cls, fields)))
}

fn jvm_getName0(jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let (target, value_type) = {
        let arg0 = args.get(0).unwrap();
        let arg0 = util::oop::extract_ref(arg0);
        let arg0 = arg0.read().unwrap();
        match &arg0.v {
            oop::RefKind::Mirror(mirror) => (mirror.target.clone(), mirror.value_type),
            _ => unreachable!(),
        }
    };
    let name = {
        match target {
            Some(target) => {
                let cls = target.read().unwrap();
                cls.name.clone()
            }
            None => match value_type {
                ValueType::INT => Arc::new(Vec::from("int")),
                _ => unimplemented!(),
            },
        }
    };

    let name = String::from_utf8_lossy(name.as_slice());
    let name = name.replace("/", ".");
    let v = util::oop::new_java_lang_string2(jt, &name);
    Ok(Some(v))
}

fn jvm_forName0(jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let arg0 = args.get(0).unwrap();
    let java_name = util::oop::extract_str(arg0);
    let initialize = {
        let arg1 = args.get(1).unwrap();
        util::oop::extract_int(arg1) != 0
    };
    let java_cls_loader = args.get(2).unwrap();
    {
        match java_cls_loader {
            Oop::Null => (),
            _ => unimplemented!("app class loader, unimpl"),
        }
    }

    let _caller_mirror = args.get(3).unwrap();

    if java_name.contains("/") {
        let msg = Some(java_name);
        let ex = runtime::exception::new(jt, classfile::consts::J_CLASS_NOT_FOUND, msg);
        return Err(ex);
    }

    let java_name = java_name.replace(".", "/");
    let cls = {
        if java_name == "sun/nio/cs/ext/ExtendedCharsets" {
            //fixme: skip, cause jvm start very slow
            None
        } else {
            require_class3(None, java_name.as_bytes())
        }
    };

    match cls {
        Some(cls) => {
            {
                let mut cls = cls.write().unwrap();
                cls.init_class(jt);
                //                trace!("finish init_class: {}", String::from_utf8_lossy(*c));
            }

            if initialize {
                oop::class::init_class_fully(jt, cls.clone());
            }

            let mirror = { cls.read().unwrap().get_mirror() };

            Ok(Some(mirror))
        }
        None => {
            error!("forName0, NotFound: {}", java_name);
            let msg = Some(java_name);
            let ex = runtime::exception::new(jt, classfile::consts::J_CLASS_NOT_FOUND, msg);
            Err(ex)
        }
    }
}

fn jvm_isPrimitive(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let v = args.get(0).unwrap();
    let v = util::oop::extract_ref(v);
    let v = v.read().unwrap();
    let v = match &v.v {
        oop::RefKind::Mirror(mirror) => {
            if mirror.target.is_none() {
                1
            } else {
                0
            }
        }
        _ => unreachable!(),
    };
    Ok(Some(Oop::new_int(v)))
}

fn jvm_isAssignableFrom(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let l = args.get(0).unwrap();
    let r = args.get(1).unwrap();

    let (lt, ltyp) = {
        let l = util::oop::extract_ref(l);
        let v = l.read().unwrap();
        match &v.v {
            oop::RefKind::Mirror(mirror) => (mirror.target.clone(), mirror.value_type),
            _ => unreachable!(),
        }
    };

    let (rt, rtyp) = {
        let r = util::oop::extract_ref(r);
        let v = r.read().unwrap();
        match &v.v {
            oop::RefKind::Mirror(mirror) => (mirror.target.clone(), mirror.value_type),
            _ => unreachable!(),
        }
    };

    let v = if lt.is_none() && rt.is_none() {
        if ltyp == rtyp {
            1
        } else {
            0
        }
    } else {
        let lt = lt.unwrap();
        let rt = rt.unwrap();
        if runtime::cmp::instance_of(rt, lt) {
            1
        } else {
            0
        }
    };

    Ok(Some(Oop::new_int(v)))
}

fn jvm_isInterface(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let v = args.get(0).unwrap();
    let v = util::oop::extract_ref(v);
    let v = v.read().unwrap();
    let v = match &v.v {
        oop::RefKind::Mirror(mirror) => match &mirror.target {
            Some(target) => {
                if target.read().unwrap().is_interface() {
                    1
                } else {
                    0
                }
            }
            None => 0,
        },
        _ => unreachable!(),
    };
    Ok(Some(Oop::new_int(v)))
}

fn jvm_getDeclaredConstructors0(jt: &mut JavaThread, env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    get_declared_method_helper(true, jt, env, args)
}

pub fn jvm_getModifiers(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let v = args.get(0).unwrap();
    let v = util::oop::extract_ref(v);
    let v = v.read().unwrap();
    let v = match &v.v {
        oop::RefKind::Mirror(mirror) => match &mirror.target {
            Some(target) => target.read().unwrap().acc_flags,
            None => acc::ACC_ABSTRACT | acc::ACC_FINAL | acc::ACC_PUBLIC,
        },
        _ => unreachable!(),
    };

    Ok(Some(Oop::new_int(v as i32)))
}

fn jvm_getSuperclass(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let mirror = args.get(0).unwrap();
    let mirror = util::oop::extract_ref(mirror);
    let v = mirror.read().unwrap();
    match &v.v {
        oop::RefKind::Mirror(mirror) => match &mirror.target {
            Some(target) => {
                let cls = target.read().unwrap();
                match &cls.super_class {
                    Some(super_cls) => {
                        let cls = super_cls.read().unwrap();
                        let mirror = cls.get_mirror();
                        Ok(Some(mirror))
                    }
                    None => Ok(Some(oop::consts::get_null())),
                }
            }
            None => Ok(Some(oop::consts::get_null())),
        },
        _ => unreachable!(),
    }
}

fn jvm_isArray(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let v = args.get(0).unwrap();

    let mirror_cls = {
        let v = util::oop::extract_ref(v);
        let v = v.read().unwrap();
        match &v.v {
            oop::RefKind::Mirror(mirror) => match &mirror.target {
                Some(target) => target.clone(),
                None => return Ok(Some(Oop::new_int(0))),
            },
            _ => unreachable!(),
        }
    };

    let cls = mirror_cls.read().unwrap();
    let v = match cls.kind {
        oop::class::ClassKind::Instance(_) => 0,
        oop::class::ClassKind::TypeArray(_) => 1,
        ClassKind::ObjectArray(_) => 1,
    };

    Ok(Some(Oop::new_int(v)))
}

fn jvm_getComponentType(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let cls_mirror = args.get(0).unwrap();
    let cls = {
        let cls_mirror = util::oop::extract_ref(cls_mirror);
        let cls = cls_mirror.read().unwrap();
        match &cls.v {
            oop::RefKind::Mirror(mirror) => mirror.target.clone().unwrap(),
            _ => unreachable!(),
        }
    };
    let cls = cls.read().unwrap();
    let v = match &cls.kind {
        oop::class::ClassKind::TypeArray(type_ary_cls) => {
            let vt = type_ary_cls.value_type.into();
            let key = unsafe { std::str::from_utf8_unchecked(vt) };
            let mirrors = PRIM_MIRROS.read().unwrap();
            mirrors.get(key).map(|it| it.clone())
        }
        oop::class::ClassKind::ObjectArray(obj_ary_cls) => {
            let component = obj_ary_cls.component.clone().unwrap();
            let cls = component.read().unwrap();
            Some(cls.get_mirror())
        }
        _ => unreachable!(),
    };
    Ok(v)
}

fn jvm_getEnclosingMethod0(jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let mirror = args.get(0).unwrap();
    let target = {
        let mirror = util::oop::extract_ref(mirror);
        let v = mirror.read().unwrap();
        match &v.v {
            oop::RefKind::Mirror(mirror) => mirror.target.clone(),
            _ => return Ok(Some(oop::consts::get_null())),
        }
    };

    let (cls_file, em) = match target {
        Some(target) => {
            let cls = target.read().unwrap();
            match &cls.kind {
                ClassKind::Instance(cls) => match &cls.enclosing_method {
                    Some(em) => (cls.class_file.clone(), em.clone()),
                    None => return Ok(Some(oop::consts::get_null())),
                },
                _ => return Ok(Some(oop::consts::get_null())),
            }
        }
        None => return Ok(Some(oop::consts::get_null())),
    };

    //push EnclosingMethod class mirror
    if em.class_index == 0 {
        panic!();
    }
    let em_class = require_class2(em.class_index, &cls_file.cp).unwrap();
    let em_class_mirror = {
        let cls = em_class.read().unwrap();
        cls.get_mirror()
    };
    let mut elms = Vec::with_capacity(3);
    elms.push(em_class_mirror);

    //push EnclosingMethod name&desc
    if em.method_index != 0 {
        let (name, desc) = constant_pool::get_name_and_type(&cls_file.cp, em.method_index as usize);
        let name = name.unwrap();
        let name = unsafe { std::str::from_utf8_unchecked(name.as_slice()) };
        let desc = desc.unwrap();
        let desc = unsafe { std::str::from_utf8_unchecked(desc.as_slice()) };
        elms.push(util::oop::new_java_lang_string2(jt, name));
        elms.push(util::oop::new_java_lang_string2(jt, desc));
    } else {
        elms.push(oop::consts::get_null());
        elms.push(oop::consts::get_null());
    }

    let ary = require_class3(None, b"[Ljava/lang/Object;").unwrap();
    let ary = Oop::new_ref_ary2(ary, elms);

    Ok(Some(ary))
}

fn jvm_getDeclaringClass0(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let mirror = args.get(0).unwrap();
    let target = {
        let mirror = util::oop::extract_ref(mirror);
        let v = mirror.read().unwrap();
        match &v.v {
            oop::RefKind::Mirror(mirror) => mirror.target.clone(),
            _ => return Ok(Some(oop::consts::get_null())),
        }
    };

    let (cls_file, target, inner_classes) = match target {
        Some(target) => {
            let cls = target.read().unwrap();
            match &cls.kind {
                ClassKind::Instance(cls) => match &cls.inner_classes {
                    Some(inner_classes) => (
                        cls.class_file.clone(),
                        target.clone(),
                        inner_classes.clone(),
                    ),
                    None => return Ok(Some(oop::consts::get_null())),
                },
                _ => return Ok(Some(oop::consts::get_null())),
            }
        }
        None => return Ok(Some(oop::consts::get_null())),
    };

    for it in inner_classes.iter() {
        if it.inner_class_info_index == 0 {
            continue;
        }

        let inner_class = require_class2(it.inner_class_info_index, &cls_file.cp).unwrap();

        if Arc::ptr_eq(&inner_class, &target) {
            return if it.outer_class_info_index == 0 {
                Ok(Some(oop::consts::get_null()))
            } else {
                let outer_class = require_class2(it.outer_class_info_index, &cls_file.cp).unwrap();
                let v = outer_class.read().unwrap();
                Ok(Some(v.get_mirror()))
            };
        }
    }

    return Ok(Some(oop::consts::get_null()));
}

fn jvm_isInstance(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let target = args.get(0).unwrap();
    let obj = args.get(1).unwrap();

    let target_cls = {
        let v = util::oop::extract_ref(target);
        let v = v.read().unwrap();
        match &v.v {
            oop::RefKind::Inst(inst) => inst.class.clone(),
            oop::RefKind::Mirror(mirror) => mirror.target.clone().unwrap(),
            _ => unreachable!(),
        }
    };
    let obj_cls = {
        let obj = util::oop::extract_ref(obj);
        let v = obj.read().unwrap();
        match &v.v {
            oop::RefKind::Inst(inst) => inst.class.clone(),
            _ => unreachable!(),
        }
    };

    let v = if runtime::cmp::instance_of(obj_cls, target_cls) {
        1
    } else {
        0
    };

    Ok(Some(Oop::new_int(v)))
}

fn jvm_getDeclaredMethods0(jt: &mut JavaThread, env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    get_declared_method_helper(false, jt, env, args)
}

fn jvm_getInterfaces0(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let mirror = {
        let arg0 = args.get(0).unwrap();
        extract_mirror_target(arg0)
    };

    let v = mirror.read().unwrap();
    let elms = match &v.kind {
        oop::ClassKind::Instance(inst) => {
            let mut elms = Vec::with_capacity(inst.class_file.interfaces.len());
            let cp = &inst.class_file.cp;
            inst.class_file.interfaces.iter().for_each(|it| {
                let cls = require_class2(*it, cp).unwrap();
                let cls = cls.read().unwrap();
                elms.push(cls.get_mirror());
            });

            elms
        }
        ClassKind::ObjectArray(_ary) => {
            let cls_cloneable = require_class3(None, cls_file_const::J_CLONEABLE).unwrap();
            let cls_serializable = require_class3(None, cls_file_const::J_SERIALIZABLE).unwrap();
            let mut elms = Vec::with_capacity(2);

            {
                let cls = cls_cloneable.read().unwrap();
                elms.push(cls.get_mirror());
            }

            {
                let cls = cls_serializable.read().unwrap();
                elms.push(cls.get_mirror());
            }

            elms
        }
        ClassKind::TypeArray(_) => unimplemented!("type array getInterfaces0"),
    };

    let clazz = require_class3(None, b"[Ljava/lang/Class;").unwrap();
    let ary = Oop::new_ref_ary2(clazz, elms);

    Ok(Some(ary))
}

fn jvm_getRawAnnotations(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let cls = args.get(0).unwrap();
    let annotations = match cls {
        Oop::Ref(rf) => {
            let rf = rf.read().unwrap();
            match &rf.v {
                oop::RefKind::Mirror(mirror) => {
                    let cls = mirror.target.clone().unwrap();
                    let cls = cls.read().unwrap();
                    match &cls.kind {
                        oop::ClassKind::Instance(cls) => {
                            match &cls.attr_runtime_visible_annotations_raw {
                                Some(v) => Oop::new_byte_ary2(v.to_vec()),
                                None => oop::consts::get_null(),
                            }
                        }
                        _ => unimplemented!(),
                    }
                }
                _ => unimplemented!(),
            }
        }
        _ => oop::consts::get_null(),
    };
    Ok(Some(annotations))
}

fn jvm_getConstantPool(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let this = args.get(0).unwrap();
    let cp_oop = match this {
        Oop::Ref(rf) => {
            let rf = rf.read().unwrap();
            match &rf.v {
                oop::RefKind::Mirror(_mirror) => {
                    let cp_cls = require_class3(None, b"sun/reflect/ConstantPool").unwrap();
                    let cp_oop = Oop::new_inst(cp_cls.clone());

                    let cls = cp_cls.read().unwrap();
                    let fid = cls.get_field_id(b"constantPoolOop", b"Ljava/lang/Object;", false);
                    //todo: reimpl maybe, create one JNIHandles, like jdk
                    cls.put_field_value(cp_oop.clone(), fid, this.clone());

                    cp_oop
                }
                _ => unimplemented!(),
            }
        }
        _ => oop::consts::get_null(),
    };

    // unreachable!();
    Ok(Some(cp_oop))
}

fn jvm_getDeclaredClasses0(_jt: &mut JavaThread, _env: JNIEnv, args: Vec<Oop>) -> JNIResult {
    let _this = args.get(0).unwrap();
    //todo: impl me
    let r = oop::consts::get_null();
    Ok(Some(r))
}

fn get_declared_method_helper(
    want_constructor: bool,
    jt: &mut JavaThread,
    _env: JNIEnv,
    args: Vec<Oop>,
) -> JNIResult {
    //parse args
    let mirror_target = {
        let arg0 = args.get(0).unwrap();
        extract_mirror_target(arg0)
    };

    let arg1 = args.get(1).unwrap();
    let public_only = util::oop::extract_int(arg1) == 1;

    //fixme: super methods
    let all_methods = {
        let cls = mirror_target.read().unwrap();
        match &cls.kind {
            oop::class::ClassKind::Instance(inst) => inst.all_methods.clone(),
            oop::class::ClassKind::ObjectArray(_ary) => HashMap::new(),
            t => unreachable!("{:?}", t),
        }
    };

    fn select_method(want_constructor: bool, m: &MethodIdRef) -> bool {
        return if want_constructor {
            m.method.name.as_slice() == b"<init>" && !m.method.is_static()
        } else {
            m.method.name.as_slice() != b"<init>"
        };
    }

    let mut selected_methods = Vec::new();
    for (_, m) in all_methods {
        if select_method(want_constructor, &m) {
            if !public_only || m.method.is_public() {
                selected_methods.push(m.clone());
            }
        }
    }

    //build methods ary
    let mut methods = Vec::new();
    for m in selected_methods {
        let v = if want_constructor {
            common::reflect::new_method_ctor(jt, m)
        } else {
            common::reflect::new_method_normal(jt, m)
        };

        methods.push(v);
    }

    //build oop methods ary
    let ary_cls = if want_constructor {
        require_class3(None, b"[Ljava/lang/reflect/Constructor;").unwrap()
    } else {
        require_class3(None, b"[Ljava/lang/reflect/Method;").unwrap()
    };

    Ok(Some(Oop::new_ref_ary2(ary_cls, methods)))
}

fn extract_mirror_target(v: &Oop) -> ClassRef {
    let v = util::oop::extract_ref(v);
    let v = v.read().unwrap();
    match &v.v {
        oop::RefKind::Mirror(mirror) => mirror.target.clone().unwrap(),
        _ => unreachable!(),
    }
}
