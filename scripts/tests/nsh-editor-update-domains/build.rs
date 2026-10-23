use napi_derive_backend_ohos::{
    NapiObject, NapiStruct, NapiStructField, NapiStructKind, TryToTokens,
};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::{env, fs, path::PathBuf};
use syn::Item;
fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_document_input)");
    let root =
        PathBuf::from(env::var_os("NSH_EDITOR_TEST_SOURCE").expect("NSH_EDITOR_TEST_SOURCE"));
    println!("cargo:rerun-if-env-changed=NSH_EDITOR_TEST_SOURCE");
    let sdk_path = root.join("crates/nsh-platform-ohos/src/interop/sdk.rs");
    let input_path = root.join("crates/nsh-platform-ohos/src/input/editor.rs");
    let queue_path = root.join("crates/nsh-platform-ohos/src/render/arkui.rs");
    for p in [&sdk_path, &input_path, &queue_path] {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    let sdk = syn::parse_file(&fs::read_to_string(sdk_path).unwrap()).unwrap();
    let mut generated = TokenStream::new();
    let state_path = root.join("crates/nsh-platform-ohos/src/input/editor_state.rs");
    println!("cargo:rerun-if-changed={}", state_path.display());
    let state_path = state_path.to_str().expect("Rust source path must be UTF-8");
    generated.extend(quote! {#[path=#state_path]mod editor_state;});
    let names = [
        "EditorSettings",
        "EditorFrame",
        "EditorValue",
        "EditorUpdate",
        "EditorAccessibility",
        "EditorCommand",
        "Batch",
    ];
    for item in sdk.items {
        match item {
            Item::Struct(mut item) if names.contains(&item.ident.to_string().as_str()) => {
                item.attrs.retain(|a| !a.path().is_ident("napi"));
                generated.extend(quote! {#item});
                if item.ident == "Batch" {
                    continue;
                }
                let fields = item
                    .fields
                    .iter()
                    .map(|f| {
                        let name = f.ident.as_ref().unwrap();
                        let mut parts = name
                            .to_string()
                            .split('_')
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        for p in &mut parts[1..] {
                            p[0..1].make_ascii_uppercase();
                        }
                        NapiStructField {
                            name: syn::Member::Named(name.clone()),
                            js_name: parts.concat(),
                            ty: f.ty.clone(),
                            getter: true,
                            setter: true,
                            writable: true,
                            enumerable: true,
                            configurable: true,
                            comments: vec![],
                            skip_typescript: false,
                            ts_type: None,
                            has_lifetime: false,
                        }
                    })
                    .collect();
                let model = NapiStruct {
                    name: item.ident.clone(),
                    js_name: item.ident.to_string(),
                    comments: vec![],
                    js_mod: None,
                    use_nullable: false,
                    register_name: syn::Ident::new("unused", Span::call_site()),
                    kind: NapiStructKind::Object(NapiObject {
                        fields,
                        object_from_js: false,
                        object_to_js: true,
                        is_tuple: false,
                    }),
                    has_lifetime: false,
                    is_generator: false,
                    is_async_generator: false,
                };
                let code = syn::parse2::<syn::File>(model.try_to_token_stream().unwrap()).unwrap();
                for item in code.items {
                    if let Item::Impl(imp) = &item {
                        if imp.trait_.as_ref().is_some_and(|(_, p, _)| {
                            p.segments.last().unwrap().ident == "ToNapiValue"
                        }) {
                            generated.extend(quote! {#item});
                        }
                    }
                }
            }
            Item::Impl(item)
                if ["EditorUpdate", "Batch"]
                    .iter()
                    .any(|n| matches!(&*item.self_ty,syn::Type::Path(p) if p.path.is_ident(n))) =>
            {
                generated.extend(quote! {#item})
            }
            _ => {}
        }
    }
    let source = syn::parse_file(&fs::read_to_string(input_path).unwrap()).unwrap();
    let keep = [
        "new",
        "target_id",
        "listen",
        "configure",
        "update",
        "editor_frame",
        "editor_value",
        "value",
        "content",
        "context",
        "retire",
        "close",
        "is_secure",
        "is_custom",
        "document_admitted",
        "promote_document",
        "with_child",
        "command",
        "focus",
        "selection",
    ];
    for item in source.items {
        match item {
            Item::Struct(item) if item.ident == "Input" => {
                if item
                    .fields
                    .iter()
                    .any(|field| field.ident.as_ref().is_some_and(|name| name == "document"))
                {
                    println!("cargo:rustc-cfg=has_document_input");
                }
                generated.extend(quote! {#item});
            }
            Item::Impl(mut item) if matches!(&*item.self_ty,syn::Type::Path(p) if p.path.is_ident("Input")) =>
            {
                item.items.retain(|m|matches!(m,syn::ImplItem::Fn(f) if keep.contains(&f.sig.ident.to_string().as_str())));
                generated.extend(quote! {#item});
            }
            Item::Fn(item) if item.sig.ident == "content_type" => generated.extend(quote! {#item}),
            _ => {}
        }
    }
    let source = syn::parse_file(&fs::read_to_string(queue_path).unwrap()).unwrap();
    for item in source.items {
        if let Item::Impl(mut item) = item {
            item.items.retain(|m|matches!(m,syn::ImplItem::Fn(f) if ["editor_update","editor_command","editor_remove","retire_unattached","editor_wake"].contains(&f.sig.ident.to_string().as_str())));
            generated.extend(quote! {#item});
        }
    }
    // Exercise the actual outer context-clear and target-rotation order as well.
    let mount_path = PathBuf::from(env::var_os("NSH_EDITOR_TEST_SOURCE").unwrap())
        .join("crates/nsh-platform-ohos/src/render/mount.rs");
    println!("cargo:rerun-if-changed={}", mount_path.display());
    let mount = syn::parse_file(&fs::read_to_string(mount_path).unwrap()).unwrap();
    for item in mount.items {
        if let Item::Impl(mut imp) = item {
            if !matches!(&*imp.self_ty,syn::Type::Path(p) if p.path.is_ident("Mounter")) {
                continue;
            }
            imp.items
                .retain(|m| matches!(m,syn::ImplItem::Fn(f) if f.sig.ident=="native_context"));
            if !imp.items.is_empty() {
                generated.extend(quote! {#imp});
            }
        }
    }
    // Other flush effects are outside this harness; retain the real ready guard.
    let queue = syn::parse_file(
        &fs::read_to_string(root.join("crates/nsh-platform-ohos/src/render/arkui.rs")).unwrap(),
    )
    .unwrap();
    for item in queue.items {
        if let Item::Impl(imp) = item {
            for m in imp.items {
                if let syn::ImplItem::Fn(f) = m {
                    if f.sig.ident == "flush_arkui" {
                        let guard = f.block.stmts.first().unwrap();
                        generated.extend(quote!{impl Mounter {fn flush_arkui(&self)->Result<()>{
                #guard
                self.applied.borrow_mut().push(std::mem::take(&mut *self.arkui.borrow_mut()));Ok(())
            }}});
                    }
                }
            }
        }
    }
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("production.rs"),
        generated.to_string(),
    )
    .unwrap();
}
