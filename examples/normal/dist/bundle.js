
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
        
define('/Users/chencheng/Documents/Code/test/toy-mako/examples/normal/index.ts', function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", {
    value: true
});
var _foots = require("/Users/chencheng/Documents/Code/test/toy-mako/examples/normal/foo.ts");
var x = (0, _foots.add(1, 2));
console.log(x);

});
define('/Users/chencheng/Documents/Code/test/toy-mako/examples/normal/foo.ts', function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", {
    value: true
});
Object.defineProperty(exports, "add", {
    enumerable: true,
    get: function() {
        return add;
    }
});
function add(a, b) {
    return a + b;
}

});
requireModule('/Users/chencheng/Documents/Code/test/toy-mako/examples/normal/index.ts');