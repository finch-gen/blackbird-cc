use std::string::ToString;
use std::path::PathBuf;
use std::str::FromStr;
use clang::*;
use rand::random;
use proc_macro2::TokenStream;
use quote::{quote, format_ident, ToTokens};
use std::fs;
use std::io::prelude::*;

trait ToTokenStream {
  fn to_token_stream(&self) -> TokenStream;
}

impl ToTokenStream for Type<'_> {
  fn to_token_stream(&self) -> TokenStream {
    let root = self.get_canonical_type();

    match root.get_kind() {
      TypeKind::Pointer => {
        let pointee = root.get_pointee_type().unwrap();
        let tokens = pointee.to_token_stream();
        if pointee.is_const_qualified() {
          quote!(*const #tokens)
        } else {
          quote!(*mut #tokens)
        }
      },

      TypeKind::Void => quote!(std::os::raw::c_void),
      TypeKind::CharS => quote!(std::os::raw::c_char),
      TypeKind::CharU => quote!(std::os::raw::c_char),
      TypeKind::SChar => quote!(std::os::raw::c_schar),
      TypeKind::UChar => quote!(std::os::raw::c_uchar),
      TypeKind::Short => quote!(std::os::raw::c_short),
      TypeKind::UShort => quote!(std::os::raw::c_ushort),
      TypeKind::Int => quote!(std::os::raw::c_int),
      TypeKind::UInt => quote!(std::os::raw::c_uint),
      TypeKind::Long => quote!(std::os::raw::c_long),
      TypeKind::ULong => quote!(std::os::raw::c_ulong),
      TypeKind::LongLong => quote!(std::os::raw::c_longlong),
      TypeKind::ULongLong => quote!(std::os::raw::c_ulonglong),
      TypeKind::Float => quote!(std::os::raw::c_float),
      TypeKind::Double => quote!(std::os::raw::c_double),
      _ => panic!("invalid type: {}", root.get_display_name()),
    }
  }
}

#[derive(Debug)]
enum Item {
  Mod(ItemMod),
  Fn(ItemFn),
  Struct(ItemStruct),
}

impl ToTokens for Item {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    match self {
      Self::Fn(item) => item.to_tokens(tokens),
      Self::Mod(item) => item.to_tokens(tokens),
      Self::Struct(item) => item.to_tokens(tokens),
    }
  }
}

#[derive(Debug)]
struct ItemMod {
  name: String,
  items: Vec<Item>,
  comments: Vec<String>,
}

impl ToTokens for ItemMod {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let name = format_ident!("{}", self.name);
    let items = &self.items;
    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());

    quote!(#(#comments)* mod #name { #(#items)* }).to_tokens(tokens);
  }
}

#[derive(Debug, Clone)]
struct Arg(Option<String>, TokenStream);

impl ToTokens for Arg {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    if let Some(name) = &self.0 {
      let name = format_ident!("{}", name);
      let ty = &self.1;
      quote!(#name: #ty).to_tokens(tokens);
    } else {
      self.1.to_tokens(tokens);
    }
  }
}

#[derive(Debug)]
struct ItemFn {
  name: String,
  symbol: String,
  args: Vec<Arg>,
  ret: TokenStream,
  comments: Vec<String>,
}

impl ToTokens for ItemFn {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let name = format_ident!("{}", self.name);
    let symbol = &self.symbol;
    let ret = &self.ret;

    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());
    let arguments = &self.args;

    quote!(
      extern {
        #(#comments)*
        #[link_name=#symbol]
        pub fn #name(#(#arguments),*) -> #ret;
      }
    ).to_tokens(tokens);
  }
}

#[derive(Debug, Clone)]
struct Field(bool, String, TokenStream);

impl ToTokens for Field {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let vis = if self.0 {
      quote!(pub)
    } else {
      TokenStream::new()
    };
    let name = format_ident!("{}", self.1);
    let ty = &self.2;
    quote!(#vis #name: #ty).to_tokens(tokens);
  }
}

#[derive(Debug, Clone)]
struct Constructor {
  name: String,
  symbol: String,
  args: Vec<Arg>,
  comments: Vec<String>,
}

impl ToTokens for Constructor {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let class = format_ident!("{}", self.name);
    let symbol = &self.symbol;

    let id = format_ident!("_{:x}", random::<u64>());

    let args = &self.args;

    let mut raw_args = vec![Arg(Some("this".to_string()), quote!(*mut #class))];
    raw_args.extend(args.clone());

    let mut arg_names = vec![quote!(&mut this as *mut #class)];
    arg_names.extend(self.args.iter().map(|x| {
      let ident = format_ident!("{}", x.0.as_ref().unwrap());
      quote!(#ident)
    }));

    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());

    quote!(
      extern {
        #[link_name=#symbol]
        fn #id(#(#raw_args),*);
      }
      impl #class {
        #(#comments)*
        pub unsafe fn new(#(#args),*) -> #class {
          let mut this = #class::default();
          #id(#(#arg_names),*);
          this
        }
      }
    ).to_tokens(tokens);
  }
}

#[derive(Debug, Clone)]
struct Destructor {
  name: String,
  symbol: String,
  comments: Vec<String>,
}

impl ToTokens for Destructor {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let class = format_ident!("{}", self.name);
    let symbol = &self.symbol;

    let id = format_ident!("_{:x}", random::<u64>());

    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());

    quote!(
      extern {
        #[link_name=#symbol]
        fn #id(_: *mut #class);
      }
      impl Drop for #class {
        #(#comments)*
        fn drop(&mut self) {
          unsafe { #id(self as *mut #class); }
        }
      }
    ).to_tokens(tokens);
  }
}

#[derive(Debug, Clone)]
struct Method {
  class: String,
  name: String,
  symbol: String,
  args: Vec<Arg>,
  ret: TokenStream,
  comments: Vec<String>,
}

impl ToTokens for Method {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let name = format_ident!("{}", self.name);
    let class = format_ident!("{}", self.class);
    let symbol = &self.symbol;
    let ret = &self.ret;

    let id = format_ident!("_{:x}", random::<u64>());

    let mut args = vec![Arg(None, quote!(&mut self))];
    args.extend(self.args.clone());

    let mut raw_args = vec![Arg(Some("this".to_string()), quote!(*mut #class))];
    raw_args.extend(self.args.clone());

    let mut arg_names = vec![quote!(self as *mut #class)];
    arg_names.extend(self.args.iter().map(|x| {
      let ident = format_ident!("{}", x.0.as_ref().unwrap());
      quote!(#ident)
    }));

    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());

    quote!(
      extern {
        #[link_name=#symbol]
        fn #id(#(#raw_args),*) -> #ret;
      }
      impl #class {
        #(#comments)*
        pub unsafe fn #name(#(#args),*) -> #ret {
          #id(#(#arg_names),*)
        }
      }
    ).to_tokens(tokens);
  }
}

#[derive(Debug, Clone)]
struct StaticMethod {
  class: String,
  name: String,
  symbol: String,
  args: Vec<Arg>,
  ret: TokenStream,
  comments: Vec<String>,
}

impl ToTokens for StaticMethod {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let name = format_ident!("{}", self.name);
    let class = format_ident!("{}", self.class);
    let id = format_ident!("_{:x}", random::<u64>());
    let symbol = &self.symbol;
    let ret = &self.ret;

    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());
    let arguments = &self.args;
    let arg_names = self.args.iter().map(|x| x.0.as_ref().unwrap());

    quote!(
      extern {
        #[link_name=#symbol]
        fn #id(#(#arguments),*) -> #ret;
      }

      impl #class {
        #(#comments)*
        pub unsafe fn #name(#(#arguments),*) -> #ret {
          #id(#(#arg_names),*)
        }
      }
    ).to_tokens(tokens);
  }
}

#[derive(Debug)]
struct ItemStruct {
  name: String,
  fields: Vec<Field>,
  constructor: Option<Constructor>,
  destructor: Option<Destructor>,
  methods: Vec<Method>,
  static_methods: Vec<StaticMethod>,
  comments: Vec<String>,
}

impl ToTokens for ItemStruct {
  fn to_tokens(&self, tokens: &mut TokenStream) {
    let name = format_ident!("{}", self.name);
    let fields = self.fields.iter().map(|x| x.to_token_stream());
    let methods = self.methods.iter().map(|x| x.to_token_stream());
    let static_methods = self.static_methods.iter().map(|x| x.to_token_stream());

    let comments = self.comments.iter().map(|x| TokenStream::from_str(&x).unwrap());

    let constructor = if let Some(constructor) = &self.constructor {
      constructor.to_token_stream()
    } else {
      TokenStream::new()
    };

    let destructor = if let Some(destructor) = &self.destructor {
      destructor.to_token_stream()
    } else {
      TokenStream::new()
    };

    quote!(
      #(#comments)*
      #[repr(C)]
      #[derive(Default, Debug)]
      pub struct #name {
        #(#fields),*
      }

      #constructor
      #destructor

      #(#methods)*
      #(#static_methods)*
    ).to_tokens(tokens);
  }
}

#[derive(Debug)]
struct State {
  glue: String,
}

#[derive(Debug, Clone)]
struct Context {
  ns: Vec<String>,
}

impl State {
  fn process_children(&mut self, e: Entity, c: &Context) -> Vec<Item> {
    let mut items = Vec::new();
    for child in e.get_children() {
      items.extend(self.process_entity(child, c))
    }
    items
  }

  fn process_entity(&mut self, e: Entity, c: &Context) -> Vec<Item> {
    match e.get_kind() {
      EntityKind::TranslationUnit => {
        self.process_children(e, c)
      },

      EntityKind::Namespace => {
        let mut c = c.clone();
        c.ns.push(e.get_name().unwrap());
        vec![Item::Mod(ItemMod {
          name: e.get_name().unwrap(),
          items: self.process_children(e, &c),
          comments: e.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
        })]
      }

      EntityKind::FunctionDecl => {
        let mut symbol = if cfg!(unix) {
          e.get_mangled_name().unwrap()[1..].to_string()
        } else {
          e.get_mangled_name().unwrap()
        };

        if e.is_inline_function() {
          symbol = format!("_{:x}", random::<u64>());
          self.glue += &format!(
            "extern \"C\" {{ {ret} {temp}({args}) {{ return {name}({arg_names}); }} }}",
            ret=e.get_result_type().unwrap().get_display_name(),
            temp=symbol,
            name=e.get_name().unwrap(),
            args=e.get_arguments().unwrap().iter().map(|arg| {
              format!("{} {}", arg.get_type().unwrap().get_display_name().to_string(), arg.get_display_name().unwrap())
            }).collect::<Vec<_>>().join(", "),
            arg_names=e.get_arguments().unwrap().iter().map(|arg| {
              arg.get_display_name().unwrap()
            }).collect::<Vec<_>>().join(", "),
          );
        }

        vec![Item::Fn(ItemFn {
          name: e.get_name().unwrap(),
          symbol: symbol,
          ret: e.get_result_type().unwrap().to_token_stream(),
          comments: e.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
          args: e.get_arguments().unwrap().iter().enumerate().map(|(i, arg)| {
            Arg(Some(arg.get_display_name().unwrap_or(format!("a{}", i))), arg.get_type().unwrap().to_token_stream())
          }).collect(),
        })]
      },

      EntityKind::ClassDecl => {
        let mut strukt = ItemStruct {
          name: e.get_name().unwrap(),
          comments: e.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
          fields: Vec::new(),
          methods: Vec::new(),
          static_methods: Vec::new(),
          constructor: None,
          destructor: None,
        };
    
        for child in e.get_children() {
          println!("{:#?}", child);

          match child.get_kind() {
            EntityKind::FieldDecl => {
              println!("{:#?}", child.get_accessibility());
              strukt.fields.push(Field(child.get_accessibility().unwrap() == Accessibility::Public, child.get_name().unwrap(), child.get_type().unwrap().to_token_stream()));
            },

            EntityKind::Constructor => {
              let symbol = if cfg!(unix) {
                child.get_mangled_name().unwrap()[1..].to_string()
              } else {
                child.get_mangled_name().unwrap()
              };

              strukt.constructor = Some(Constructor {
                name: child.get_name().unwrap(),
                symbol,
                args: child.get_arguments().unwrap().iter().enumerate().map(|(i, arg)| {
                  Arg(Some(arg.get_display_name().unwrap_or(format!("a{}", i))), arg.get_type().unwrap().to_token_stream())
                }).collect(),
                comments: child.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
              });
            },

            EntityKind::Destructor => {
              let symbol = if cfg!(unix) {
                child.get_mangled_names().unwrap()[0][1..].to_string()
              } else {
                child.get_mangled_names().unwrap()[0].clone()
              };

              strukt.destructor = Some(Destructor {
                name: e.get_name().unwrap(),
                symbol,
                comments: child.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
              });
            },

            EntityKind::Method => {
              let symbol = if cfg!(unix) {
                child.get_mangled_name().unwrap()[1..].to_string()
              } else {
                child.get_mangled_name().unwrap()
              };

              if child.is_static_method() {
                strukt.static_methods.push(StaticMethod {
                  class: e.get_name().unwrap(),
                  name: child.get_name().unwrap(),
                  symbol,
                  args: child.get_arguments().unwrap().iter().enumerate().map(|(i, arg)| {
                    Arg(Some(arg.get_display_name().unwrap_or(format!("a{}", i))), arg.get_type().unwrap().to_token_stream())
                  }).collect(),
                  ret: child.get_result_type().unwrap().to_token_stream(),
                  comments: child.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
                });
              } else {
                strukt.methods.push(Method {
                  class: e.get_name().unwrap(),
                  name: child.get_name().unwrap(),
                  symbol,
                  args: child.get_arguments().unwrap().iter().enumerate().map(|(i, arg)| {
                    Arg(Some(arg.get_display_name().unwrap_or(format!("a{}", i))), arg.get_type().unwrap().to_token_stream())
                  }).collect(),
                  ret: child.get_result_type().unwrap().to_token_stream(),
                  comments: child.get_comment().map_or(Vec::new(), |x| x.split("\n").map(|x| x.to_string()).collect()),
                });
              }
            },

            _ => {},
          }
        }

        vec![Item::Struct(strukt)]
      }

      _ => Vec::new(),
    }
  }
}

pub fn generate<P: Into<PathBuf>>(path: P) {
  let path = path.into();

  let clang = Clang::new().unwrap();

  let index = Index::new(&clang, false, false);

  let args = vec!["-std=c++11"];
  let tu = index.parser(&path).arguments(&args).parse().unwrap();
  let entity = tu.get_entity();

  let mut state = State {
    glue: String::new(),
  };

  let items = state.process_entity(entity, &Context {
    ns: Vec::new(),
  });

  let mut tokens = TokenStream::new();
  for item in items {
    item.to_tokens(&mut tokens);
  }

  let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

  let mut f = fs::File::create(out_dir.join("bindings.rs")).unwrap();
  f.write_fmt(format_args!("{}", tokens)).unwrap();

  let mut f = fs::File::create(out_dir.join("glue.cc")).unwrap();
  f.write_fmt(format_args!("#include \"{}\"\n", path.to_str().unwrap().to_string().replace("\\", "\\\\"))).unwrap();
  f.write_fmt(format_args!("{}", state.glue)).unwrap();
  drop(f);

  cc::Build::new()
    .file(out_dir.join("glue.cc"))
    .compile("glue");
}
