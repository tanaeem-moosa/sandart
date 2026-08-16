// Automated static and runtime scope checker for sandart-wasm/web/demo.js
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const jsPath = path.resolve(__dirname, '../sandart-wasm/web/demo.js');
console.log(`Checking ${jsPath}...`);

let code = fs.readFileSync(jsPath, 'utf8');
// Replace ES module imports with no-op in VM sandbox
code = code.replace(/import\s+.*?from\s+['"].*?['"];?/g, '// import removed');

// Mock browser DOM and WASM environment for initialization validation
const mockElements = new Map();
function getMockElement(id) {
    if (!mockElements.has(id)) {
        mockElements.set(id, {
            id,
            value: '0',
            innerText: '',
            textContent: '',
            style: {},
            dataset: {},
            classList: {
                add() {},
                remove() {},
                toggle() {},
                contains() { return false; }
            },
            addEventListener(event, handler) {},
            querySelectorAll() { return []; },
            appendChild() {},
            cloneNode() { return getMockElement(id + '_clone'); },
            getBoundingClientRect() { return { width: 800, height: 600 }; }
        });
    }
    return mockElements.get(id);
}

const mockDocument = {
    getElementById(id) {
        return getMockElement(id);
    },
    querySelectorAll(selector) {
        return [];
    },
    createElement(tag) {
        return getMockElement('created_' + tag);
    }
};

const mockWasmState = {
    list_materials() {
        return [
            ['dry_sand', 'Dry Sand', 0.0, 0.5, 0.8, 1.0],
            ['water', 'Water', 1.0, 0.0, 1.0, 0.0]
        ];
    },
    build_git_sha() { return 'test-sha'; },
    build_timestamp_epoch() { return 1700000000; },
    get_grid_size() { return 128; },
    get_multistage_chambers() { return 8; },
    neck_half_width_cells() { return 2.0; },
    set_sandbox_shape() {},
    set_simulator_mode() {},
    set_gravity() {},
    set_neck_width() {},
    set_hourglass_curve() {},
    set_multistage_chambers() {},
    set_quantile_mode() {},
    set_pattern_mode() {},
    load_preset_pattern() {},
    set_active_color_theme() {},
    set_primary_palette() {},
    set_secondary_palette() {},
    set_cell_props() {},
    reset() {},
    flip_hourglass() {},
    update() {},
    render() {}
};

const sandbox = {
    console,
    window: {
        addEventListener() {},
        devicePixelRatio: 1.0,
        innerWidth: 800,
        innerHeight: 600
    },
    document: mockDocument,
    performance: { now: () => 0 },
    requestAnimationFrame() {},
    WasmSimulationState: mockWasmState,
    init: async () => {},
    alert(msg) { console.warn("Alert called:", msg); },
    Float32Array,
    Uint8Array,
    Uint32Array,
    Map,
    Set,
    Math,
    parseInt,
    parseFloat,
    String,
    Date
};

vm.createContext(sandbox);

try {
    vm.runInContext(code, sandbox);
    console.log("✓ Syntax and top-level execution PASSED");

    // Test calling functions to ensure all internal references exist
    if (typeof sandbox.switchMode === 'function') {
        sandbox.switchMode('sandfall');
        sandbox.switchMode('sandbox');
        console.log("✓ switchMode() execution PASSED");
    }

    if (typeof sandbox.populateMaterialSelects === 'function') {
        sandbox.populateMaterialSelects();
        console.log("✓ populateMaterialSelects() execution PASSED");
    }

    if (typeof sandbox.updateChambersRowVisibility === 'function') {
        sandbox.updateChambersRowVisibility();
        console.log("✓ updateChambersRowVisibility() execution PASSED");
    }

    if (typeof sandbox.updateVesselReadouts === 'function') {
        sandbox.updateVesselReadouts();
        console.log("✓ updateVesselReadouts() execution PASSED");
    }

    console.log("All JS pre-commit checks PASSED successfully!");
    process.exit(0);
} catch (err) {
    console.error("❌ JS Pre-commit validation FAILED:", err);
    process.exit(1);
}
