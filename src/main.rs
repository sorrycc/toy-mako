use std::sync::Arc;
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug)]
struct Module {
    ast: Ast,
    deps: HashMap<String, PathBuf>,
}

#[derive(Debug)]
struct Ast {
    ast: SwcModule,
    unresolved_mark: Mark,
    top_level_mark: Mark,
}

struct ModuleGraph {
    modules: HashMap<String, Module>,
}

struct Context {
    root: PathBuf,
    cm: Lrc<SourceMap>,
    comments: SwcComments,
    globals: Globals,
}

struct CompileParams {
    root: PathBuf,
}

fn compile(params: CompileParams) {
    let root = params.root;
    let context = Arc::new(Context {
        root: root.clone(),
        cm: Default::default(),
        comments: Default::default(),
        globals: Default::default(),
    });
    let mut module_graph = build(BuildParams {
        path: root.join("index.ts"),
        context: context.clone(),
    });
    generate(&mut module_graph, context.clone());
}

/////////////////////////////////////////
// Build Stage

struct BuildQueue {
    queue: Vec<PathBuf>,
}

struct BuildParams {
    path: PathBuf,
    context: Arc<Context>,
}

fn build(params: BuildParams) -> ModuleGraph {
    let mut build_queue = BuildQueue {queue: vec![params.path],};
    let mut module_graph = ModuleGraph {modules: HashMap::new(),};
    let context = params.context;
    while let Some(path) = build_queue.queue.pop() {
        if module_graph.modules.contains_key(&path.to_string_lossy().to_string()) { continue; }
        // load
        let content = load(&path);
        // parse
        let mut ast = parse(content, &path, context.clone());
        // transform
        transform(&mut ast, context.clone());
        // analyze_deps
        let deps = analyze_deps(&ast);
        // resolve
        let mut hash_deps = HashMap::<String, PathBuf>::new();
        deps.iter().for_each(|dep| {
            let resolved = resolve(&path, dep);
            hash_deps.insert(dep.to_string(), resolved);
        });
        module_graph.modules.insert(
            path.to_string_lossy().to_string(),
            Module {
                ast,
                deps: hash_deps.clone(),
            },
        );
        build_queue.queue.extend(hash_deps.into_values());
    }
    module_graph
}

fn load(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn parse(content: String, path: &PathBuf, context: Arc<Context>) -> Ast {
    let ast = code_to_ast(content, path, context.cm.clone(), &context.comments);
    GLOBALS.set(&context.globals, || Ast {
        ast,
        unresolved_mark: Mark::new(),
        top_level_mark: Mark::new(),
    })
}

fn transform(ast: &mut Ast, context: Arc<Context>) {
    let unresolved_mark = ast.unresolved_mark;
    let top_level_mark = ast.top_level_mark;
    let ast = &mut ast.ast;
    let comments = &context.comments;
    GLOBALS.set(&context.globals, || {
        HELPERS.set(&Helpers::new(true), || {
            let resolver = resolver(unresolved_mark, top_level_mark, false);
            let preset_env = preset_env::preset_env(
                unresolved_mark,
                Some(comments),
                Default::default(),
                Default::default(),
                &mut Default::default(),
            );
            let body = ast.body.take();
            let module = SwcModule {
                span: ast.span,
                shebang: ast.shebang.clone(),
                body,
            };
            let mut folders = chain!(resolver, preset_env);
            ast.body = folders.fold_module(module).body;
        });
    });
}

fn analyze_deps(ast: &Ast) -> Vec<String> {
    let mut deps = vec![];
    ast.ast.body.iter().for_each(|item| {
        if let ModuleItem::ModuleDecl(decl) = item {
            if let ModuleDecl::Import(import) = decl {
                deps.push(import.src.value.to_string());
            }
        }
    });
    deps
}

fn resolve(path: &PathBuf, dep: &str) -> PathBuf {
    use oxc_resolver::{ResolveOptions, Resolver};
    let resolver = Resolver::new(ResolveOptions {
        extensions: vec![".ts".to_string()],
        ..Default::default()
    });
    let resolved = resolver.resolve(path.parent().unwrap(), dep).unwrap();
    resolved.full_path()
}

/////////////////////////////////////////
// Generate Stage

fn generate(module_graph: &mut ModuleGraph, context: Arc<Context>) {
    // TODO:
    // - tree shaking
    // - skip modules & module concatenation
    // - chunk group and splitting
    // - source map
    // - minification
    // - parallel
    // - ...

    let mut runtime = Runtime {
        modules: HashMap::new(),
    };
    let module_paths = module_graph.modules.keys().cloned().collect::<Vec<_>>();
    for path in module_paths {
        let module = module_graph.modules.get_mut(&path).unwrap();
        replace_deps(&mut module.ast, &module.deps);
        tramsform_again(&mut module.ast, context.clone());
        let code = ast_to_code(&module.ast, context.clone());
        runtime.modules.insert(path.to_string(), code);
    }
    let code = runtime.render(context.clone());
    // write to disk
    let output_dir = context.root.join("dist");
    std::fs::create_dir_all(&output_dir).unwrap();
    std::fs::write(output_dir.join("bundle.js"), code).unwrap();
}

struct Runtime {
    modules: HashMap<String, String>,
}

impl Runtime {
    fn render(&self, context: Arc<Context>) -> String {
        let mut ret = vec![];
        ret.push(
            r#"
const modules = new Map();
const define = (name, moduleFactory) => {
  modules.set(name, moduleFactory);
};

const moduleCache = new Map();
const requireModule = (name) => {
  if (moduleCache.has(name)) {
    return moduleCache.get(name).exports;
  }

  if (!modules.has(name)) {
    throw new Error(`Module '${name}' does not exist.`);
  }

  const moduleFactory = modules.get(name);
  const module = {
    exports: {},
  };
  moduleCache.set(name, module);
  moduleFactory(module, module.exports, requireModule);
  return module.exports;
};
        "#
            .to_string(),
        );
        self.modules.iter().for_each(|(path, code)| {
            ret.push(format!(
                "define('{}', function (module, exports, require) {{\n{}\n}});",
                path, code
            ));
        });
        ret.push(format!(
            "requireModule('{}');",
            context.root.join("index.ts").to_string_lossy()
        ));
        ret.join("\n")
    }
}

fn tramsform_again(ast: &mut Ast, context: Arc<Context>) {
    GLOBALS.set(&context.globals, || {
        HELPERS.set(&Helpers::new(true), || {
            ast.ast
                .visit_mut_with(&mut inject_helpers(ast.unresolved_mark));
            ast.ast.visit_mut_with(&mut common_js(
                ast.unresolved_mark,
                Default::default(),
                FeatureFlag::empty(),
                Some(&context.comments),
            ));
        });
    });
}

fn replace_deps(ast: &mut Ast, deps: &HashMap<String, PathBuf>) {
    ast.ast.body.iter_mut().for_each(|item| {
        if let ModuleItem::ModuleDecl(decl) = item {
            if let ModuleDecl::Import(import) = decl {
                let dep = import.src.value.to_string();
                if let Some(path) = deps.get(&dep) {
                    import.src = Box::new(path.to_string_lossy().to_string().into());
                }
            }
        }
    });
}

/////////////////////////////////////////
// Utils

use swc_core::{
    common::{
        chain, input::StringInput, sync::Lrc, util::take::Take, FileName, Globals, Mark, SourceMap,
        GLOBALS,
    },
    ecma::{
        ast::{Module as SwcModule, *},
        codegen::{self, text_writer::JsWriter, Emitter},
        parser::{lexer::Lexer, Parser, Syntax},
        preset_env,
        transforms::{
            base::{
                feature::FeatureFlag,
                helpers::inject_helpers,
                helpers::{Helpers, HELPERS},
                resolver,
            },
            module::common_js,
        },
        visit::{Fold, VisitMutWith},
    },
};
use swc_node_comments::SwcComments;

fn code_to_ast(
    code: String,
    path: &PathBuf,
    cm: Lrc<SourceMap>,
    comments: &SwcComments,
) -> SwcModule {
    let path = path.to_string_lossy().to_string();
    let file = cm.new_source_file(FileName::Custom(path.into()), code);
    let lexer = Lexer::new(
        Syntax::Es(Default::default()),
        EsVersion::latest(),
        StringInput::from(&*file),
        Some(&comments),
    );
    let mut parser = Parser::new_from(lexer);
    let module = parser.parse_module().unwrap();
    module
}

fn ast_to_code(ast: &Ast, context: Arc<Context>) -> String {
    let mut buf = vec![];
    let mut source_map_buf = Vec::new();
    {
        let mut emitter = Emitter {
            cfg: codegen::Config::default(),
            cm: context.cm.clone(),
            comments: Some(&context.comments),
            wr: Box::new(JsWriter::new(
                context.cm.clone(),
                "\n",
                &mut buf,
                Some(&mut source_map_buf),
            )),
        };
        emitter.emit_module(&ast.ast).unwrap();
    }
    let code = String::from_utf8(buf).unwrap();
    code
}

/////////////////////////////////////////
// Main

fn main() {
    let root = std::env::current_dir().unwrap().join("examples/normal");
    compile(CompileParams { root });
    println!("Done!");
    println!("Run `node examples/normal/dist/bundle.js` to see the result.");
}
