const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { chromium, firefox, webkit } = require("playwright");

const root = path.resolve(__dirname, "../../..");
const defaultOutput = path.join(
  root,
  "testing/browser/artifacts/runner-observation.json",
);

function command(program, arguments = []) {
  return execFileSync(program, arguments, { encoding: "utf8" }).trim();
}

function read(file) {
  return fs.readFileSync(file, "utf8").trim();
}

function cpuRecords() {
  return read("/proc/cpuinfo")
    .split(/\n\s*\n/)
    .filter(Boolean)
    .map((record) =>
      Object.fromEntries(
        record.split("\n").map((line) => {
          const separator = line.indexOf(":");
          return [line.slice(0, separator).trim(), line.slice(separator + 1).trim()];
        }),
      ),
    );
}

function physicalMemoryBytes() {
  const output = command("sudo", ["-n", "dmidecode", "--type", "memory"]);
  const sizes = [...output.matchAll(/^\s*Size:\s+(\d+)\s+(MB|GB)$/gm)];
  if (sizes.length === 0) {
    throw new Error("dmidecode did not report installed memory modules");
  }
  return sizes.reduce((total, [, amount, unit]) => {
    const multiplier = unit === "GB" ? 1024 ** 3 : 1024 ** 2;
    return total + Number(amount) * multiplier;
  }, 0);
}

function observedAffinity() {
  const output = command("taskset", ["-pc", String(process.pid)]);
  const separator = output.lastIndexOf(":");
  if (separator < 0) {
    throw new Error(`unexpected taskset output: ${output}`);
  }
  return output.slice(separator + 1).trim();
}

function readUniformCpuSetting(relative) {
  const values = fs
    .readdirSync("/sys/devices/system/cpu")
    .filter((entry) => /^cpu\d+$/.test(entry))
    .map((cpu) => read(path.join("/sys/devices/system/cpu", cpu, relative)));
  if (values.length === 0 || new Set(values).size !== 1) {
    throw new Error(`CPU setting ${relative} is missing or nonuniform`);
  }
  return values[0];
}

function acPowerPresent() {
  const root = "/sys/class/power_supply";
  const supplies = fs.existsSync(root) ? fs.readdirSync(root) : [];
  const mains = supplies.filter(
    (supply) => read(path.join(root, supply, "type")) === "Mains",
  );
  if (mains.length > 0) {
    return mains.some((supply) => read(path.join(root, supply, "online")) === "1");
  }
  return !supplies.some(
    (supply) => read(path.join(root, supply, "type")) === "Battery",
  );
}

function versionWithoutProgram(program, arguments = []) {
  return command(program, arguments).replace(/^\S+\s+/, "");
}

async function browserObservation(name, browserType) {
  const browser = await browserType.launch({ headless: true });
  try {
    const executable = browserType.executablePath().replaceAll("\\", "/");
    const revision = executable.match(new RegExp(`${name}-(\\d+)/`))?.[1];
    if (!revision) {
      throw new Error(`cannot read ${name} revision from ${executable}`);
    }
    return { version: browser.version(), revision };
  } finally {
    await browser.close();
  }
}

async function observeRunner() {
  const cpus = cpuRecords();
  const physicalCores = new Set(
    cpus.map((cpu) => `${cpu["physical id"]}:${cpu["core id"]}`),
  ).size;
  const release = Object.fromEntries(
    read("/etc/os-release")
      .split("\n")
      .filter((line) => line.includes("="))
      .map((line) => {
        const separator = line.indexOf("=");
        return [
          line.slice(0, separator),
          line.slice(separator + 1).replace(/^"|"$/g, ""),
        ];
      }),
  );
  const libcLine = command("ldd", ["--version"]).split("\n")[0];
  const libcVersion = libcLine.match(/(\d+\.\d+)\s*$/)?.[1];
  if (!libcVersion) {
    throw new Error(`cannot parse libc version from ${libcLine}`);
  }
  const installedTargets = new Set(command("rustup", ["target", "list", "--installed"]).split("\n"));
  if (!installedTargets.has("wasm32-unknown-unknown")) {
    throw new Error("wasm32-unknown-unknown is not installed");
  }
  const affinity = observedAffinity();
  const [observedChromium, observedFirefox, observedWebkit] = await Promise.all([
    browserObservation("chromium", chromium),
    browserObservation("firefox", firefox),
    browserObservation("webkit", webkit),
  ]);
  const wasmOpt = command("wasm-opt", ["--version"]);
  const wasmOptVersion = wasmOpt.match(/version\s+(\d+)/)?.[1];
  if (!wasmOptVersion) {
    throw new Error(`cannot parse wasm-opt version from ${wasmOpt}`);
  }
  const noTurbo = read("/sys/devices/system/cpu/intel_pstate/no_turbo");
  return {
    version: 1,
    environment: "schemaform-perf-v1",
    comparison_policy: "Abort before measurement when any pinned value differs.",
    hardware: {
      architecture: process.arch === "x64" ? "x86_64" : process.arch,
      cpu_vendor: cpus[0].vendor_id,
      cpu_model: cpus[0]["model name"],
      physical_cores: physicalCores,
      logical_cpus: os.cpus().length,
      microcode: cpus[0].microcode,
      memory_bytes: physicalMemoryBytes(),
    },
    operating_system: {
      distribution: release.PRETTY_NAME,
      kernel: os.release(),
      libc: `glibc ${libcVersion}`,
    },
    browsers: {
      playwright: require("playwright/package.json").version,
      chromium: observedChromium,
      firefox: observedFirefox,
      webkit: observedWebkit,
    },
    rust_wasm_tools: {
      rust: versionWithoutProgram("rustc", ["--version"]),
      cargo: versionWithoutProgram("cargo", ["--version"]),
      wasm_target: "wasm32-unknown-unknown",
      wasm_bindgen_cli: versionWithoutProgram("wasm-bindgen", ["--version"]),
      binaryen_wasm_opt: wasmOptVersion,
      wasm_tools: versionWithoutProgram("wasm-tools", ["--version"]),
    },
    compression: {
      brotli: versionWithoutProgram("brotli", ["--version"]),
      quality: 11,
      window: 22,
    },
    power: {
      governor: readUniformCpuSetting("cpufreq/scaling_governor"),
      turbo: noTurbo === "1" ? "disabled" : "enabled",
      intel_pstate_min_perf_pct: Number(
        read("/sys/devices/system/cpu/intel_pstate/min_perf_pct"),
      ),
      intel_pstate_max_perf_pct: Number(
        read("/sys/devices/system/cpu/intel_pstate/max_perf_pct"),
      ),
      ac_power_required: acPowerPresent(),
    },
    affinity: {
      runner_process_cpu_list: affinity,
      browser_process_cpu_list: affinity,
      measurement_cpu_list: affinity,
    },
    production_artifact: readJson(
      path.join(root, "testing/browser/workload-pack/runner-manifest.json"),
    ).production_artifact,
    empty_shell_artifact: readJson(
      path.join(root, "testing/browser/workload-pack/runner-manifest.json"),
    ).empty_shell_artifact,
  };
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

async function main() {
  const output = path.resolve(process.env.RUNNER_OBSERVATION ?? defaultOutput);
  const observation = await observeRunner();
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(observation, null, 2)}\n`);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}

module.exports = { observeRunner };
