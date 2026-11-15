//! Procedural macros of Rango: `#[derive(Model)]`, `filter!`, `q!`.
//!
//! ```ignore
//! #[derive(Model)]
//! pub struct Question {
//!     #[field(primary_key, auto)] pub id: Option<i64>,
//!     #[field(max_length = 200)] pub question_text: String,
//!     #[field(auto_now_add)] pub pub_date: DateTime<Utc>,
//! }
//!
//! Question::objects().filter(filter!(question_text__startswith = "What")).fetch(&db).await?;
//! ```

use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Ident, Span, TokenTree};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, ExprLit, Fields, GenericArgument, Lit, PathArguments, Type, parse_macro_input};

// ---------------------------------------------------------------------------------- derive

#[derive(Default)]
struct ModelAttrs {
    krate: Option<String>,
    app_label: Option<String>,
    db_table: Option<String>,
    ordering: Vec<String>,
    verbose_name: Option<String>,
    verbose_name_plural: Option<String>,
    str_field: Option<String>,
    unique_together: Vec<Vec<String>>,
    default_related_name: Option<String>,
    get_latest_by: Vec<String>,
    managed: Option<bool>,
}

#[derive(Default, Clone)]
struct FieldAttrs {
    primary_key: bool,
    auto: bool,
    big_auto: bool,
    max_length: Option<u32>,
    max_digits: Option<u32>,
    decimal_places: Option<u32>,
    null: bool,
    blank: bool,
    unique: bool,
    db_index: bool,
    auto_now: bool,
    auto_now_add: bool,
    db_column: Option<String>,
    verbose_name: Option<String>,
    help_text: Option<String>,
    upload_to: Option<String>,
    related_name: Option<String>,
    related_query_name: Option<String>,
    on_delete: Option<String>,
    to_field: Option<String>,
    through: Option<String>,
    default: Option<Expr>,
    kind: Option<String>,
    editable: Option<bool>,
    state: bool,
    skip: bool,
}

fn lit_str(e: &Expr) -> Option<String> {
    match e {
        Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => Some(s.value()),
        _ => None,
    }
}

fn str_list(e: &Expr) -> Vec<String> {
    match e {
        Expr::Array(a) => a.elems.iter().filter_map(lit_str).collect(),
        other => lit_str(other).into_iter().collect(),
    }
}

fn parse_model_attrs(input: &DeriveInput) -> syn::Result<ModelAttrs> {
    let mut a = ModelAttrs::default();
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("model")) {
        attr.parse_nested_meta(|m| {
            let key = m.path.get_ident().map(ToString::to_string).unwrap_or_default();
            if key == "crate" || key == "r#crate" {
                a.krate = lit_str(&m.value()?.parse::<Expr>()?);
                return Ok(());
            }
            let e: Expr = m.value()?.parse()?;
            match key.as_str() {
                "app_label" => a.app_label = lit_str(&e),
                "db_table" => a.db_table = lit_str(&e),
                "ordering" => a.ordering = str_list(&e),
                "verbose_name" => a.verbose_name = lit_str(&e),
                "verbose_name_plural" => a.verbose_name_plural = lit_str(&e),
                "str" => a.str_field = lit_str(&e),
                "default_related_name" => a.default_related_name = lit_str(&e),
                "get_latest_by" => a.get_latest_by = str_list(&e),
                "managed" => a.managed = Some(matches!(&e, Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) if b.value)),
                "unique_together" => {
                    if let Expr::Array(outer) = &e {
                        a.unique_together = outer.elems.iter().map(str_list).collect();
                    }
                }
                other => return Err(m.error(format!("unknown model option `{other}`"))),
            }
            Ok(())
        })?;
    }
    Ok(a)
}

fn parse_field_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs> {
    let mut f = FieldAttrs::default();
    for attr in attrs {
        if attr.path().is_ident("model") {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("state") {
                    f.state = true;
                } else if m.path.is_ident("skip") {
                    f.skip = true;
                }
                Ok(())
            })?;
            continue;
        }
        if !attr.path().is_ident("field") {
            continue;
        }
        attr.parse_nested_meta(|m| {
            let key = m.path.get_ident().map(ToString::to_string).unwrap_or_default();
            let flag = |m: &syn::meta::ParseNestedMeta| m.input.peek(syn::Token![=]);
            if !flag(&m) {
                match key.as_str() {
                    "primary_key" => f.primary_key = true,
                    "auto" => f.auto = true,
                    "big_auto" => f.big_auto = true,
                    "null" => f.null = true,
                    "blank" => f.blank = true,
                    "unique" => f.unique = true,
                    "db_index" => f.db_index = true,
                    "auto_now" => f.auto_now = true,
                    "auto_now_add" => f.auto_now_add = true,
                    "text" | "email" | "url" | "slug" | "json" | "binary" | "ip" | "file" | "image" => f.kind = Some(key),
                    other => return Err(m.error(format!("unknown field option `{other}`"))),
                }
                return Ok(());
            }
            let e: Expr = m.value()?.parse()?;
            let int = |e: &Expr| match e {
                Expr::Lit(ExprLit { lit: Lit::Int(i), .. }) => i.base10_parse::<u32>().ok(),
                _ => None,
            };
            match key.as_str() {
                "max_length" => f.max_length = int(&e),
                "max_digits" => f.max_digits = int(&e),
                "decimal_places" => f.decimal_places = int(&e),
                "db_column" => f.db_column = lit_str(&e),
                "verbose_name" => f.verbose_name = lit_str(&e),
                "help_text" => f.help_text = lit_str(&e),
                "upload_to" => f.upload_to = lit_str(&e),
                "related_name" => f.related_name = lit_str(&e),
                "related_query_name" => f.related_query_name = lit_str(&e),
                "on_delete" => f.on_delete = lit_str(&e),
                "to_field" => f.to_field = lit_str(&e),
                "through" => f.through = lit_str(&e),
                "default" => f.default = Some(e),
                "editable" => f.editable = Some(matches!(&e, Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) if b.value)),
                other => return Err(m.error(format!("unknown field option `{other}`"))),
            }
            Ok(())
        })?;
    }
    Ok(f)
}

/// `(base ident, generic arg)` of a path type: `Option<String>` -> ("Option", Some(String)).
fn type_parts(ty: &Type) -> (String, Option<Type>) {
    if let Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            let name = seg.ident.to_string();
            if let PathArguments::AngleBracketed(ab) = &seg.arguments {
                if let Some(GenericArgument::Type(t)) = ab.args.first() {
                    return (name, Some(t.clone()));
                }
            }
            return (name, None);
        }
    }
    (String::new(), None)
}

fn snake_to_lower(name: &str) -> String {
    name.to_lowercase()
}

#[derive(PartialEq)]
enum Kind {
    Plain,
    Fk { one_to_one: bool },
    M2m,
    State,
}

struct F<'a> {
    ident: &'a Ident,
    name: String,
    ty: &'a Type,
    inner: Option<Type>,
    nullable: bool,
    attrs: FieldAttrs,
    kind: Kind,
    target: Option<Type>,
}

#[proc_macro_derive(Model, attributes(model, field))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_model(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_model(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let attrs = parse_model_attrs(input)?;
    let name = &input.ident;
    let Data::Struct(data) = &input.data else { return Err(syn::Error::new(input.span(), "#[derive(Model)] needs a struct")) };
    let Fields::Named(named) = &data.fields else { return Err(syn::Error::new(input.span(), "#[derive(Model)] needs named fields")) };
    let krate: syn::Path = syn::parse_str(attrs.krate.as_deref().unwrap_or("::rango::db"))?;
    let object_name = name.to_string();
    let fields_ident = format_ident!("{}Fields", name);

    let mut fs: Vec<F> = Vec::new();
    for f in &named.named {
        let ident = f.ident.as_ref().expect("named");
        let fa = parse_field_attrs(&f.attrs)?;
        let (base, arg) = type_parts(&f.ty);
        let (nullable, inner_ty) = if base == "Option" { (true, arg.clone()) } else { (false, None) };
        let eff_ty = inner_ty.clone().unwrap_or_else(|| f.ty.clone());
        let (eb, earg) = type_parts(&eff_ty);
        let kind = if fa.state || base == "ModelState" {
            Kind::State
        } else if eb == "ForeignKey" || eb == "OneToOne" {
            Kind::Fk { one_to_one: eb == "OneToOne" }
        } else if eb == "ManyToMany" {
            Kind::M2m
        } else {
            Kind::Plain
        };
        let target = if matches!(kind, Kind::Fk { .. } | Kind::M2m) { earg } else { None };
        fs.push(F { ident, name: ident.to_string(), ty: &f.ty, inner: inner_ty, nullable, attrs: fa, kind, target });
    }

    // ---- ModelMeta builder
    let app_label_expr = match &attrs.app_label {
        Some(a) => quote! { #a.to_owned() },
        None => quote! {{
            let mp: Vec<&str> = module_path!().split("::").collect();
            let n = mp.len();
            (if n >= 2 && mp[n - 1] == "models" { mp[n - 2] } else { mp[0] }).to_owned()
        }},
    };
    let mut field_builders = Vec::new();
    for f in &fs {
        if matches!(f.kind, Kind::State) || f.attrs.skip {
            continue;
        }
        let fname = &f.name;
        let a = &f.attrs;
        let eff = f.inner.clone().unwrap_or_else(|| f.ty.clone());
        let (base, _) = type_parts(&eff);
        let mut ctor = match f.kind {
            Kind::Fk { one_to_one } => {
                let t = f.target.as_ref().expect("fk target");
                let od = match a.on_delete.as_deref().unwrap_or("cascade") {
                    "protect" => quote! { OnDelete::Protect },
                    "restrict" => quote! { OnDelete::Restrict },
                    "set_null" => quote! { OnDelete::SetNull },
                    "set_default" => quote! { OnDelete::SetDefault },
                    "do_nothing" => quote! { OnDelete::DoNothing },
                    _ => quote! { OnDelete::Cascade },
                };
                if one_to_one {
                    quote! { Field::one_to_one(#fname, &<#t as #krate::models::Model>::label(), #od) }
                } else {
                    quote! { Field::foreign_key(#fname, &<#t as #krate::models::Model>::label(), #od) }
                }
            }
            Kind::M2m => {
                let t = f.target.as_ref().expect("m2m target");
                quote! { Field::many_to_many(#fname, &<#t as #krate::models::Model>::label()) }
            }
            _ => {
                let ft = if a.auto {
                    quote! { FieldType::Auto }
                } else if a.big_auto {
                    quote! { FieldType::BigAuto }
                } else {
                    match (a.kind.as_deref(), base.as_str()) {
                        (Some("text"), _) => quote! { FieldType::Text },
                        (Some("email"), _) => quote! { FieldType::Email },
                        (Some("url"), _) => quote! { FieldType::Url },
                        (Some("slug"), _) => quote! { FieldType::Slug },
                        (Some("json"), _) => quote! { FieldType::Json },
                        (Some("binary"), _) => quote! { FieldType::Binary },
                        (Some("ip"), _) => quote! { FieldType::GenericIpAddress },
                        (Some("file"), _) => quote! { FieldType::File },
                        (Some("image"), _) => quote! { FieldType::Image },
                        (_, "String") => {
                            if a.max_length.is_some() { quote! { FieldType::Char } } else { quote! { FieldType::Text } }
                        }
                        (_, "i8" | "i16") => quote! { FieldType::SmallInteger },
                        (_, "i32") => quote! { FieldType::Integer },
                        (_, "i64" | "isize") => quote! { FieldType::BigInteger },
                        (_, "u8" | "u16") => quote! { FieldType::PositiveSmallInteger },
                        (_, "u32") => quote! { FieldType::PositiveInteger },
                        (_, "u64" | "usize") => quote! { FieldType::PositiveBigInteger },
                        (_, "bool") => quote! { FieldType::Boolean },
                        (_, "f32" | "f64") => quote! { FieldType::Float },
                        (_, "Decimal") => quote! { FieldType::Decimal },
                        (_, "DateTime") => quote! { FieldType::DateTime },
                        (_, "NaiveDateTime") => quote! { FieldType::DateTime },
                        (_, "NaiveDate") => quote! { FieldType::Date },
                        (_, "NaiveTime") => quote! { FieldType::Time },
                        (_, "TimeDelta") => quote! { FieldType::Duration },
                        (_, "Uuid") => quote! { FieldType::Uuid },
                        (_, "Vec") => quote! { FieldType::Binary },
                        (_, "Value") => quote! { FieldType::Json },
                        (_, other) => return Err(syn::Error::new(f.ty.span(), format!("cannot infer a model field type for `{other}`; use #[field(text|json|...)] or a relation wrapper"))),
                    }
                };
                quote! { Field::new(#fname, #ft) }
            }
        };
        let mut mods: Vec<proc_macro2::TokenStream> = Vec::new();
        if a.primary_key || a.auto || a.big_auto {
            mods.push(quote! { .primary_key() });
        }
        if let Some(n) = a.max_length {
            mods.push(quote! { ; f.max_length = Some(#n) });
        }
        if let Some(n) = a.max_digits {
            mods.push(quote! { ; f.max_digits = Some(#n) });
        }
        if let Some(n) = a.decimal_places {
            mods.push(quote! { ; f.decimal_places = Some(#n) });
        }
        if (f.nullable && !(a.primary_key || a.auto || a.big_auto)) || a.null {
            mods.push(quote! { .null() });
        }
        if a.blank {
            mods.push(quote! { .blank() });
        }
        if a.unique {
            mods.push(quote! { .unique() });
        }
        if a.db_index {
            mods.push(quote! { .db_index() });
        }
        if a.auto_now {
            mods.push(quote! { .auto_now() });
        }
        if a.auto_now_add {
            mods.push(quote! { .auto_now_add() });
        }
        if let Some(c) = &a.db_column {
            mods.push(quote! { .db_column(#c) });
        }
        if let Some(v) = &a.verbose_name {
            mods.push(quote! { .verbose_name(#v) });
        }
        if let Some(v) = &a.help_text {
            mods.push(quote! { .help_text(#v) });
        }
        if let Some(v) = &a.upload_to {
            mods.push(quote! { .upload_to(#v) });
        }
        if let Some(v) = &a.related_name {
            mods.push(quote! { .related_name(#v) });
        }
        if let Some(v) = &a.related_query_name {
            mods.push(quote! { .related_query_name(#v) });
        }
        if let Some(v) = &a.to_field {
            mods.push(quote! { .to_field(#v) });
        }
        if let Some(v) = &a.through {
            mods.push(quote! { .through(#v) });
        }
        if let Some(d) = &a.default {
            mods.push(quote! { .default(#d) });
        }
        if a.editable == Some(false) {
            mods.push(quote! { ; f.editable = false });
        }
        // Apply chained modifiers; `; stmt` entries mutate the field.
        let mut builder = quote! { let mut f = #ctor; };
        for m in mods {
            let s = m.to_string();
            if s.starts_with(';') {
                let stmt: proc_macro2::TokenStream = s[1..].parse().expect("tokens");
                builder.extend(quote! { #stmt; });
            } else {
                builder.extend(quote! { f = f #m; });
            }
        }
        ctor = quote! {{ #builder f }};
        field_builders.push(ctor);
    }
    let ordering = &attrs.ordering;
    let unique_together = attrs.unique_together.iter().map(|g| quote! { m.unique_together.push(vec![#(#g.to_owned()),*]); });
    let db_table = attrs.db_table.as_ref().map(|t| quote! { m.db_table = #t.to_owned(); });
    let vn = attrs.verbose_name.as_ref().map(|v| quote! { m.verbose_name = #v.to_owned(); });
    let vnp = attrs.verbose_name_plural.as_ref().map(|v| quote! { m.verbose_name_plural = #v.to_owned(); });
    let strf = attrs.str_field.as_ref().map(|v| quote! { m.str_field = Some(#v.to_owned()); });
    let drn = attrs.default_related_name.as_ref().map(|v| quote! { m.default_related_name = Some(#v.to_owned()); });
    let gl = &attrs.get_latest_by;
    let managed = attrs.managed.map(|v| quote! { m.managed = #v; });

    // ---- Model impl
    let mut from_fields = Vec::new();
    let mut to_entries = Vec::new();
    let mut pre_save = Vec::new();
    let mut pk_ident = None;
    let mut pk_plain_int = false;
    let mut state_init = Vec::new();
    for f in &fs {
        let id = f.ident;
        let fname = &f.name;
        match f.kind {
            Kind::State => {
                from_fields.push(quote! { #id: #krate::models::ModelState::default() });
                state_init.push(id);
                continue;
            }
            Kind::M2m => {
                from_fields.push(quote! { #id: #krate::models::ManyToMany::from_prefetched(d.prefetched.get(#fname))? });
                continue;
            }
            Kind::Fk { .. } => {
                let att = format!("{fname}_id");
                if f.nullable {
                    from_fields.push(quote! { #id: if d.fields.get(#att).is_none_or(#krate::Value::is_null) { None } else { Some(#krate::models::ForeignKey::from_data(d.fields.get(#att), d.related.get(#fname))?) } });
                } else {
                    from_fields.push(quote! { #id: #krate::models::ForeignKey::from_data(d.fields.get(#att), d.related.get(#fname))? });
                }
                to_entries.push(quote! { (#att.to_owned(), #krate::ToValue::to_value(&self.#id)) });
                continue;
            }
            Kind::Plain => {}
        }
        if f.attrs.skip {
            from_fields.push(quote! { #id: ::core::default::Default::default() });
            continue;
        }
        from_fields.push(quote! { #id: #krate::FromValue::from_value(d.fields.get(#fname).cloned().unwrap_or(#krate::Value::Null))? });
        to_entries.push(quote! { (#fname.to_owned(), #krate::ToValue::to_value(&self.#id)) });
        if f.attrs.primary_key || f.attrs.auto || f.attrs.big_auto {
            pk_ident = Some(id);
            let (b, _) = type_parts(f.inner.as_ref().unwrap_or(f.ty));
            pk_plain_int = !f.nullable && b != "String" && b != "Uuid";
        }
        let eff = f.inner.clone().unwrap_or_else(|| f.ty.clone());
        let (b, _) = type_parts(&eff);
        if f.attrs.auto_now || f.attrs.auto_now_add {
            let value = if b == "NaiveDate" {
                quote! { #krate::chrono_now().date_naive() }
            } else if b == "NaiveDateTime" {
                quote! { #krate::chrono_now().naive_utc() }
            } else {
                quote! { #krate::chrono_now() }
            };
            let assign = if f.nullable { quote! { self.#id = Some(#value) } } else { quote! { self.#id = #value } };
            if f.attrs.auto_now {
                pre_save.push(quote! { #assign; });
            } else {
                pre_save.push(quote! { if created { #assign; } });
            }
        }
    }
    let pk_ident = pk_ident.ok_or_else(|| syn::Error::new(input.span(), "a model needs a primary key field: #[field(primary_key, auto)] id: Option<i64>"))?;
    let pk_value = if pk_plain_int {
        quote! { match #krate::ToValue::to_value(&self.#pk_ident) { #krate::Value::Int(0) => #krate::Value::Null, v => v } }
    } else {
        quote! { #krate::ToValue::to_value(&self.#pk_ident) }
    };

    // ---- typed fields struct
    let mut field_methods = Vec::new();
    let mut reverse_accessors = Vec::new();
    for f in &fs {
        if matches!(f.kind, Kind::State) || f.attrs.skip {
            continue;
        }
        let id = f.ident;
        let fname = &f.name;
        match f.kind {
            Kind::Fk { .. } | Kind::M2m => {
                let t = f.target.as_ref().expect("target");
                field_methods.push(quote! {
                    pub fn #id(&self) -> <#t as #krate::models::Model>::Fields {
                        <<#t as #krate::models::Model>::Fields as #krate::models::typed::PrefixedFields>::with_prefix(self.join(#fname))
                    }
                });
                if matches!(f.kind, Kind::Fk { .. }) {
                    let idm = format_ident!("{}_id", id);
                    let att = format!("{fname}_id");
                    field_methods.push(quote! { pub fn #idm(&self) -> #krate::models::Path<i64> { #krate::models::Path::new(self.join(#att)) } });
                    // Reverse accessor on the target: `question.choice_set()`.
                    let accessor = match f.attrs.related_name.as_deref() {
                        Some(n) if n.ends_with('+') => None,
                        Some(n) => Some(n.to_owned()),
                        None => Some(format!("{}_set", snake_to_lower(&object_name))),
                    };
                    if let Some(acc) = accessor {
                        let acc_ident = format_ident!("{}", acc);
                        reverse_accessors.push(quote! {
                            impl #t {
                                /// Reverse relation manager (`instance.<name>_set` in the reference implementation).
                                pub fn #acc_ident(&self) -> #krate::models::QuerySet<#name> {
                                    #krate::models::QuerySet::<#name>::new().filter_q(#krate::models::Q::lookup(#fname, #krate::models::Model::pk_value(self)))
                                }
                            }
                        });
                    }
                }
                if matches!(f.kind, Kind::M2m) {
                    let acc = format_ident!("{}_manager", id);
                    let rev = f.attrs.related_query_name.clone().or(f.attrs.related_name.clone()).unwrap_or_else(|| snake_to_lower(&object_name));
                    field_methods.push(quote! {});
                    reverse_accessors.push(quote! {
                        impl #name {
                            /// Relation manager of the many-to-many field.
                            pub fn #acc(&self) -> #krate::models::ManyAccessor<#t> {
                                #krate::models::ManyAccessor {
                                    owner_label: <#name as #krate::models::Model>::label(),
                                    field: #fname,
                                    pk: #krate::models::Model::pk_value(self),
                                    reverse_query_name: #rev.to_owned(),
                                    _t: ::core::marker::PhantomData,
                                }
                            }
                        }
                    });
                }
            }
            Kind::Plain => {
                let eff = f.inner.clone().unwrap_or_else(|| f.ty.clone());
                field_methods.push(quote! { pub fn #id(&self) -> #krate::models::Path<#eff> { #krate::models::Path::new(self.join(#fname)) } });
            }
            Kind::State => {}
        }
    }
    let pk_ty = fs.iter().find(|f| f.ident == pk_ident).map(|f| f.inner.clone().unwrap_or_else(|| f.ty.clone())).expect("pk");
    let label_lit = quote! { format!("{}.{}", { #app_label_expr }, #object_name) };
    let model_meta_name = &object_name;

    Ok(quote! {
        impl #krate::models::Model for #name {
            type Fields = #fields_ident;

            fn label() -> String {
                #label_lit
            }

            fn from_data(d: #krate::models::RowData) -> #krate::Result<Self> {
                Ok(#name { #(#from_fields),* })
            }

            fn to_data(&self) -> #krate::indexmap::IndexMap<String, #krate::Value> {
                #krate::indexmap::IndexMap::from([#(#to_entries),*])
            }

            fn pk_value(&self) -> #krate::Value {
                #pk_value
            }

            fn set_pk(&mut self, v: #krate::Value) {
                if let Ok(x) = #krate::FromValue::from_value(v) {
                    self.#pk_ident = x;
                }
            }

            fn pre_save(&mut self, created: bool) {
                let _ = created;
                #(#pre_save)*
            }
        }

        #krate::inventory::submit! {
            #krate::models::meta::ModelRegistration {
                build: || {
                    use #krate::models::{Field, FieldType, OnDelete, ModelMeta};
                    let mut m = ModelMeta::new(&{ #app_label_expr }, #model_meta_name);
                    m.source = Some((file!().to_owned(), line!()));
                    #db_table
                    #vn
                    #vnp
                    #strf
                    #drn
                    #managed
                    m.ordering = vec![#(#ordering.to_owned()),*];
                    m.get_latest_by = vec![#(#gl.to_owned()),*];
                    #(#unique_together)*
                    #(m.fields.push(#field_builders);)*
                    m
                }
            }
        }

        /// Typed field handles of the model (used by `filter!` and `.filter(|f| ...)`).
        #[derive(Default, Clone, Debug)]
        pub struct #fields_ident {
            prefix: String,
        }

        impl #krate::models::typed::PrefixedFields for #fields_ident {
            fn with_prefix(prefix: String) -> Self {
                #fields_ident { prefix }
            }
            fn prefix(&self) -> &str {
                &self.prefix
            }
        }

        impl #fields_ident {
            fn join(&self, name: &str) -> String {
                if self.prefix.is_empty() { name.to_owned() } else { format!("{}__{}", self.prefix, name) }
            }

            #[allow(non_snake_case)]
            pub fn pk(&self) -> #krate::models::Path<#pk_ty> {
                #krate::models::Path::new(self.join("pk"))
            }

            /// Compare the relation itself (its primary key): `f.question().eq(3)`.
            pub fn eq(&self, v: impl #krate::ToValue) -> #krate::models::Q {
                #krate::models::Path::<()>::new(self.prefix.clone()).eq(v)
            }

            pub fn is_in<V: #krate::ToValue>(&self, v: impl IntoIterator<Item = V>) -> #krate::models::Q {
                #krate::models::Path::<()>::new(self.prefix.clone()).is_in(v)
            }

            pub fn isnull(&self, v: bool) -> #krate::models::Q {
                #krate::models::Path::<()>::new(self.prefix.clone()).isnull(v)
            }

            /// Traverse a reverse relation by its query name (`f.reverse::<ChoiceFields>("choice")`).
            pub fn reverse<R: #krate::models::typed::PrefixedFields>(&self, name: &str) -> R {
                R::with_prefix(self.join(name))
            }

            #(#field_methods)*
        }

        #(#reverse_accessors)*
    })
}

// ---------------------------------------------------------------------------------- filter!

const LOOKUPS: &[&str] = &[
    "exact", "iexact", "contains", "icontains", "in", "gt", "gte", "lt", "lte", "startswith", "istartswith", "endswith", "iendswith", "range", "isnull", "regex",
    "iregex",
];

enum Cond {
    Kv(String, proc_macro2::TokenStream),
    And(Vec<Cond>),
    Or(Vec<Cond>),
    Not(Box<Cond>),
}

struct Parser {
    toks: Vec<TokenTree>,
    pos: usize,
}

fn is_punct(t: &TokenTree, c: char) -> bool {
    matches!(t, TokenTree::Punct(p) if p.as_char() == c)
}

impl Parser {
    fn peek(&self) -> Option<&TokenTree> {
        self.toks.get(self.pos)
    }

    fn list(&mut self) -> Result<Cond, String> {
        let mut items = vec![self.or_expr()?];
        while self.peek().is_some_and(|t| is_punct(t, ',')) {
            self.pos += 1;
            if self.peek().is_none() {
                break;
            }
            items.push(self.or_expr()?);
        }
        Ok(if items.len() == 1 { items.pop().expect("one") } else { Cond::And(items) })
    }

    fn or_expr(&mut self) -> Result<Cond, String> {
        let mut items = vec![self.and_expr()?];
        while self.peek().is_some_and(|t| is_punct(t, '|')) {
            self.pos += 1;
            items.push(self.and_expr()?);
        }
        Ok(if items.len() == 1 { items.pop().expect("one") } else { Cond::Or(items) })
    }

    fn and_expr(&mut self) -> Result<Cond, String> {
        let mut items = vec![self.factor()?];
        while self.peek().is_some_and(|t| is_punct(t, '&')) {
            self.pos += 1;
            items.push(self.factor()?);
        }
        Ok(if items.len() == 1 { items.pop().expect("one") } else { Cond::And(items) })
    }

    fn factor(&mut self) -> Result<Cond, String> {
        match self.peek().cloned() {
            Some(t) if is_punct(&t, '!') => {
                self.pos += 1;
                Ok(Cond::Not(Box::new(self.factor()?)))
            }
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
                self.pos += 1;
                let mut inner = Parser { toks: g.stream().into_iter().collect(), pos: 0 };
                inner.list()
            }
            Some(TokenTree::Ident(id)) => {
                self.pos += 1;
                if !self.peek().is_some_and(|t| is_punct(t, '=')) {
                    return Err(format!("expected `=` after `{id}`"));
                }
                self.pos += 1;
                let mut value = proc_macro2::TokenStream::new();
                let mut depth_ok = true;
                while let Some(t) = self.peek() {
                    if depth_ok && (is_punct(t, ',') || is_punct(t, '&') || is_punct(t, '|')) {
                        break;
                    }
                    value.extend(std::iter::once(t.clone()));
                    self.pos += 1;
                    depth_ok = true;
                }
                if value.is_empty() {
                    return Err(format!("missing value for `{id}`"));
                }
                Ok(Cond::Kv(id.to_string(), value))
            }
            other => Err(format!("unexpected token {other:?}")),
        }
    }
}

fn emit(c: &Cond, f: &Ident) -> proc_macro2::TokenStream {
    match c {
        Cond::Kv(key, value) => {
            let segs: Vec<&str> = key.split("__").collect();
            let (path, lookup) = match segs.split_last() {
                Some((last, rest)) if !rest.is_empty() && LOOKUPS.contains(last) => (rest.to_vec(), Some(*last)),
                _ => (segs.clone(), None),
            };
            let mut chain = quote! { #f };
            for s in &path {
                let id = Ident::new_raw_safe(s);
                chain = quote! { #chain.#id() };
            }
            match lookup {
                None | Some("exact") => quote! { #chain.eq(#value) },
                Some("in") => quote! { #chain.is_in(#value) },
                Some(l) => {
                    let id = Ident::new_raw_safe(l);
                    quote! { #chain.#id(#value) }
                }
            }
        }
        Cond::And(items) => {
            let mut sorted: Vec<&Cond> = items.iter().collect();
            // the reference implementation sorts keyword arguments of one filter() call.
            sorted.sort_by_key(|c| if let Cond::Kv(k, _) = c { k.clone() } else { String::new() });
            let parts: Vec<_> = sorted.iter().map(|c| emit(c, f)).collect();
            quote! { (#(#parts)&*) }
        }
        Cond::Or(items) => {
            let parts: Vec<_> = items.iter().map(|c| emit(c, f)).collect();
            quote! { (#(#parts)|*) }
        }
        Cond::Not(inner) => {
            let i = emit(inner, f);
            quote! { (!#i) }
        }
    }
}

trait IdentExt {
    fn new_raw_safe(s: &str) -> Ident;
}
impl IdentExt for Ident {
    fn new_raw_safe(s: &str) -> Ident {
        syn::parse_str::<Ident>(s).unwrap_or_else(|_| Ident::new_raw(s, Span::call_site()))
    }
}

fn expand_cond(input: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = proc_macro2::TokenStream::from(input).into_iter().collect();
    let mut p = Parser { toks, pos: 0 };
    let cond = match p.list() {
        Ok(c) if p.pos >= p.toks.len() => c,
        Ok(_) => return syn::Error::new(Span::call_site(), "unexpected tokens in filter!").to_compile_error().into(),
        Err(e) => return syn::Error::new(Span::call_site(), e).to_compile_error().into(),
    };
    let f = Ident::new("__f", Span::call_site());
    let body = emit(&cond, &f);
    quote! { |#f| #body }.into()
}

/// `filter!(a = 1, b__gt = 2)`, `filter!((a = 1) | (b = 2))`, `filter!(!(a = 1))`: a closure over the
/// model's typed field handles, for `QuerySet::filter` / `exclude`. Misspelled fields fail to compile.
#[proc_macro]
pub fn filter(input: TokenStream) -> TokenStream {
    expand_cond(input)
}

/// Alias of [`filter!`] (the `Q(...)`).
#[proc_macro]
pub fn q(input: TokenStream) -> TokenStream {
    expand_cond(input)
}

// ---------------------------------------------------------------------------------- derive(Form)

/// `#[derive(Form)]`: a typed form. Generates `Struct::form() -> Form` (the field definitions)
/// and `Struct::from_form(&Form) -> Option<Struct>` (typed `cleaned_data` once valid).
///
/// ```ignore
/// #[derive(Form)]
/// struct Contact {
///     #[form(max_length = 100)] name: String,
///     #[form(email, label = "Your e-mail")] email: String,
///     #[form(widget = "textarea")] message: String,
///     cc_myself: bool,
///     age: Option<u32>,
/// }
/// ```
#[proc_macro_derive(Form, attributes(form))]
pub fn derive_form(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_form(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn last_segment(ty: &Type) -> Option<(String, Option<&Type>)> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    let inner = match &seg.arguments {
        PathArguments::AngleBracketed(a) => a.args.iter().find_map(|g| if let GenericArgument::Type(t) = g { Some(t) } else { None }),
        _ => None,
    };
    Some((seg.ident.to_string(), inner))
}

fn expand_form(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else { return Err(syn::Error::new(input.span(), "#[derive(Form)] needs a struct")) };
    let Fields::Named(fields) = &data.fields else { return Err(syn::Error::new(input.span(), "#[derive(Form)] needs named fields")) };
    let mut krate = quote!(::rango_forms);
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("form")) {
        attr.parse_nested_meta(|m| {
            if m.path.is_ident("crate") {
                if let Some(s) = lit_str(&m.value()?.parse::<Expr>()?) {
                    let p: syn::Path = syn::parse_str(&s)?;
                    krate = quote!(::#p);
                }
            }
            Ok(())
        })?;
    }
    let mut defs = Vec::new();
    let mut reads = Vec::new();
    for f in &fields.named {
        let ident = f.ident.as_ref().expect("named");
        let fname = ident.to_string();
        let (mut ty_name, mut inner) = last_segment(&f.ty).ok_or_else(|| syn::Error::new(f.ty.span(), "unsupported field type"))?;
        let optional = ty_name == "Option";
        if optional {
            let t = inner.ok_or_else(|| syn::Error::new(f.ty.span(), "Option needs a type argument"))?;
            (ty_name, inner) = last_segment(t).ok_or_else(|| syn::Error::new(f.ty.span(), "unsupported field type"))?;
        }
        // options
        let mut mods: Vec<proc_macro2::TokenStream> = Vec::new();
        let mut kind: Option<String> = None;
        let mut regex: Option<String> = None;
        let mut widget: Option<String> = None;
        let mut choices: Vec<(String, String)> = Vec::new();
        let mut required: Option<bool> = None;
        for attr in f.attrs.iter().filter(|a| a.path().is_ident("form")) {
            attr.parse_nested_meta(|m| {
                let key = m.path.get_ident().map(ToString::to_string).unwrap_or_default();
                if matches!(key.as_str(), "email" | "url" | "slug" | "ip" | "uuid" | "json") {
                    kind = Some(key);
                    return Ok(());
                }
                let e: Expr = m.value()?.parse()?;
                let num = |e: &Expr| -> proc_macro2::TokenStream { quote!(#e) };
                match key.as_str() {
                    "max_length" => {
                        let n = num(&e);
                        mods.push(quote!(.max_length((#n) as usize)));
                    }
                    "min_length" => {
                        let n = num(&e);
                        mods.push(quote!(.min_length((#n) as usize)));
                    }
                    "min_value" => {
                        let n = num(&e);
                        mods.push(quote!(.min_value(#n)));
                    }
                    "max_value" => {
                        let n = num(&e);
                        mods.push(quote!(.max_value(#n)));
                    }
                    "max_digits" => {
                        let n = num(&e);
                        mods.push(quote!(.max_digits((#n) as u32)));
                    }
                    "decimal_places" => {
                        let n = num(&e);
                        mods.push(quote!(.decimal_places((#n) as u32)));
                    }
                    "label" => {
                        let s = lit_str(&e);
                        mods.push(quote!(.label(#s)));
                    }
                    "help_text" => {
                        let s = lit_str(&e);
                        mods.push(quote!(.help_text(#s)));
                    }
                    "initial" => mods.push(quote!(.initial(#e))),
                    "required" => required = Some(matches!(&e, Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) if b.value)),
                    "widget" => widget = lit_str(&e),
                    "regex" => regex = lit_str(&e),
                    "choices" => {
                        if let Expr::Array(a) = &e {
                            for item in &a.elems {
                                if let Expr::Tuple(t) = item {
                                    let v: Vec<String> = t.elems.iter().filter_map(lit_str).collect();
                                    if v.len() == 2 {
                                        choices.push((v[0].clone(), v[1].clone()));
                                    }
                                }
                            }
                        }
                    }
                    other => return Err(m.error(format!("unknown form option `{other}`"))),
                }
                Ok(())
            })?;
        }
        let multiple = ty_name == "Vec";
        let base = if !choices.is_empty() {
            let items = choices.iter().map(|(v, l)| quote!(#krate::Choice::option(#v, #l)));
            if multiple { quote!(#krate::Field::multiple_choice(vec![#(#items),*])) } else { quote!(#krate::Field::choice(vec![#(#items),*])) }
        } else if let Some(k) = &kind {
            match k.as_str() {
                "email" => quote!(#krate::Field::email()),
                "url" => quote!(#krate::Field::url()),
                "slug" => quote!(#krate::Field::slug()),
                "ip" => quote!(#krate::Field::generic_ip_address("both", false)),
                "uuid" => quote!(#krate::Field::uuid()),
                _ => quote!(#krate::Field::json()),
            }
        } else if let Some(r) = &regex {
            quote!(#krate::Field::regex(#r))
        } else {
            match ty_name.as_str() {
                "String" | "str" => quote!(#krate::Field::char()),
                "bool" => quote!(#krate::Field::boolean()),
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "isize" | "usize" => quote!(#krate::Field::integer()),
                "f32" | "f64" => quote!(#krate::Field::float()),
                "Decimal" => quote!(#krate::Field::decimal()),
                "NaiveDate" => quote!(#krate::Field::date()),
                "NaiveTime" => quote!(#krate::Field::time()),
                "NaiveDateTime" | "DateTime" => quote!(#krate::Field::datetime()),
                "Uuid" => quote!(#krate::Field::uuid()),
                "Value" => quote!(#krate::Field::json()),
                other => return Err(syn::Error::new(f.ty.span(), format!("no form field for type `{other}`; add `#[form(email)]`, `choices = [...]`, etc."))),
            }
        };
        let widget_mod = widget.as_deref().map(|w| match w {
            "textarea" => quote!(.widget(#krate::Widget::textarea())),
            "password" => quote!(.widget(#krate::Widget::password_input())),
            "hidden" => quote!(.widget(#krate::Widget::hidden_input())),
            "radio" => quote!(.widget(#krate::Widget::radio_select())),
            "checkbox_multiple" => quote!(.widget(#krate::Widget::checkbox_select_multiple())),
            "select" => quote!(.widget(#krate::Widget::select())),
            "number" => quote!(.widget(#krate::Widget::number_input())),
            "email" => quote!(.widget(#krate::Widget::email_input())),
            "date" => quote!(.widget(#krate::Widget::date_input())),
            _ => quote!(),
        });
        // `bool` is a yes/no answer, so an unchecked box is valid unless `required` says otherwise.
        let req = required.unwrap_or(!(optional || ty_name == "bool" || (multiple && false)));
        defs.push(quote! { .field(#fname, #base #(#mods)* #widget_mod .required(#req)) });
        reads.push(quote! { #ident: #krate::FromFormValue::from_form_value(cleaned.get(#fname).unwrap_or(&#krate::Value::None))? });
    }
    let form_name = name.to_string();
    Ok(quote! {
        impl #name {
            /// The form definition (an unbound `Form`); bind it with `.bound(data, files)`.
            pub fn form() -> #krate::Form {
                #krate::Form::new(#form_name) #(#defs)*
            }

            /// Typed `cleaned_data`: `None` while the form is invalid or unbound.
            pub fn from_form(form: &#krate::Form) -> Option<Self> {
                if !form.is_valid() {
                    return None;
                }
                let cleaned = form.cleaned_data();
                Some(#name { #(#reads),* })
            }
        }
    })
}
