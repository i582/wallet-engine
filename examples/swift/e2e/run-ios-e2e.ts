import { execFile, spawn, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

import { DappActor, OfficialBridge } from "../../web/e2e/fixtures/processes";
import { TEST_DAPP_CONFIG } from "../../web/e2e/scenarios/definitions";

const REPOSITORY_ROOT: string = path.resolve(import.meta.dirname, "../../..");
const PROCESS_START_TIMEOUT_MS: number = 10_000;
const SCREENSHOT_DEVICE_TYPE: string =
	"com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro";
const runFile = promisify(execFile);

interface SimulatorDevice {
	deviceTypeIdentifier: string;
	isAvailable: boolean;
	name: string;
	udid: string;
}

interface SimulatorDeviceList {
	devices: Record<string, SimulatorDevice[]>;
}

interface SimulatorRuntime {
	identifier: string;
	isAvailable: boolean;
	platform: string;
	supportedDeviceTypes: Array<{ identifier: string }>;
	version: string;
}

interface SimulatorRuntimeList {
	runtimes: SimulatorRuntime[];
}

/** Owns the deterministic provider used by the iOS client scenarios. */
class ProviderActor {
	private readonly child: ChildProcess;
	private readonly output: string[] = [];
	readonly url: string;

	/** Retains process output so a failed simulator run reports provider diagnostics. */
	private constructor(child: ChildProcess, url: string) {
		this.child = child;
		this.url = url;
		child.stdout?.on("data", (chunk) => this.output.push(String(chunk)));
		child.stderr?.on("data", (chunk) => this.output.push(String(chunk)));
	}

	/** Starts the scripted provider on an isolated loopback HTTP port. */
	static async start(): Promise<ProviderActor> {
		const port: number = await freePort();
		const script: string = path.join(
			REPOSITORY_ROOT,
			"examples/web/e2e/support/provider-server.ts",
		);
		const child = spawn(
			process.execPath,
			[script, "--http", "--port", port.toString()],
			{
				stdio: ["ignore", "pipe", "pipe"],
			},
		);
		const actor = new ProviderActor(child, `http://127.0.0.1:${port}`);
		await actor.waitUntilReady();
		return actor;
	}

	/** Returns provider output captured during the complete simulator run. */
	logs(): string {
		return this.output.join("");
	}

	/** Stops the provider after xcodebuild has released the application. */
	async stop(): Promise<void> {
		if (this.child.exitCode !== null || this.child.signalCode !== null) {
			return;
		}
		this.child.kill("SIGTERM");
		await new Promise<void>((resolve) => {
			this.child.once("exit", () => resolve());
			setTimeout(() => {
				if (this.child.exitCode === null && this.child.signalCode === null) {
					this.child.kill("SIGKILL");
				}
				resolve();
			}, 2_000);
		});
	}

	/** Waits until the provider health endpoint accepts requests. */
	private async waitUntilReady(): Promise<void> {
		const deadline: number = Date.now() + PROCESS_START_TIMEOUT_MS;
		while (Date.now() < deadline) {
			if (this.child.exitCode !== null || this.child.signalCode !== null) {
				throw new Error(`Provider exited during startup\n${this.logs()}`);
			}
			try {
				const response: Response = await fetch(`${this.url}/health`);
				if (response.ok) {
					return;
				}
			} catch {
				// The listener can refuse a connection while the process is starting.
			}
			await delay(50);
		}
		throw new Error(`Timed out waiting for ${this.url}\n${this.logs()}`);
	}
}

/** Runs xcodebuild with the endpoints consumed by the UI-test launch environment. */
async function runXcodebuild(
	bridge: OfficialBridge,
	dapp: DappActor,
	provider: ProviderActor,
): Promise<number> {
	const destination: string = await resolveSimulatorDestination();
	const simulatorIdentifier: string = requiredSimulatorIdentifier(destination);
	const onlyTesting: string =
		process.env.IOS_E2E_ONLY_TESTING?.trim() || "WalletEngineAppUITests";
	const derivedDataPath: string =
		process.env.IOS_E2E_DERIVED_DATA_PATH?.trim() ||
		path.join(REPOSITORY_ROOT, "target/swift-example-ios-e2e");
	await configureSimulatorStatusBar(simulatorIdentifier);
	const arguments_: string[] = [
		"-project",
		path.join(REPOSITORY_ROOT, "examples/swift/WalletEngineApp.xcodeproj"),
		"-scheme",
		"WalletEngineApp",
		"-configuration",
		"Debug",
		"-destination",
		destination,
		"-derivedDataPath",
		derivedDataPath,
		`-only-testing:${onlyTesting}`,
		`TON_CONNECT_BRIDGE_URL=${bridge.url}`,
		`TONCENTER_BASE_URL=${provider.url}`,
		`IOS_DAPP_ORIGIN=${dapp.origin}`,
		`IOS_SNAPSHOT_SOURCE_DIR=${path.join(
			REPOSITORY_ROOT,
			"examples/swift/WalletEngineAppUITests/Snapshots",
		)}`,
		`IOS_SNAPSHOT_VARIANT=${process.env.IOS_SNAPSHOT_VARIANT ?? "iphone-16-pro"}`,
		`UPDATE_IOS_SNAPSHOTS=${process.env.UPDATE_IOS_SNAPSHOTS ?? "0"}`,
		"test",
	];
	try {
		const child = spawn("xcodebuild", arguments_, {
			env: {
				...process.env,
				IOS_DAPP_ORIGIN: dapp.origin,
				IOS_SNAPSHOT_SOURCE_DIR: path.join(
					REPOSITORY_ROOT,
					"examples/swift/WalletEngineAppUITests/Snapshots",
				),
				TONCENTER_BASE_URL: provider.url,
				TON_CONNECT_BRIDGE_URL: bridge.url,
			},
			stdio: "inherit",
		});
		return await new Promise<number>((resolve) => {
			child.once("error", (error) => {
				process.stderr.write(`Could not start xcodebuild: ${String(error)}\n`);
				resolve(1);
			});
			child.once("exit", (code) => resolve(code ?? 1));
		});
	} finally {
		await clearSimulatorStatusBar(simulatorIdentifier);
	}
}

/** Returns the simulator identifier required for deterministic visual state. */
function requiredSimulatorIdentifier(destination: string): string {
	const identifier: string | undefined =
		destination.match(/(?:^|,)id=([^,]+)/)?.[1];
	if (identifier === undefined) {
		throw new Error(
			"The iOS E2E destination must include an explicit simulator id",
		);
	}
	return identifier;
}

/** Boots the selected simulator and fixes status indicators used by visual baselines. */
async function configureSimulatorStatusBar(identifier: string): Promise<void> {
	try {
		await runFile("xcrun", ["simctl", "boot", identifier]);
	} catch (error) {
		const diagnostic: string = String(
			(error as { stderr?: string }).stderr ?? error,
		);
		if (!diagnostic.includes("Booted")) {
			throw error;
		}
	}
	await runFile("xcrun", ["simctl", "bootstatus", identifier, "-b"]);
	await runFile("xcrun", [
		"simctl",
		"status_bar",
		identifier,
		"override",
		"--time",
		"09:41",
		"--batteryState",
		"charged",
		"--batteryLevel",
		"100",
		"--cellularBars",
		"4",
		"--wifiBars",
		"3",
	]);
}

/** Restores simulator-managed status indicators after the visual test run. */
async function clearSimulatorStatusBar(identifier: string): Promise<void> {
	await runFile("xcrun", ["simctl", "status_bar", identifier, "clear"]);
}

/** Selects an existing iPhone 16 Pro or creates one on the newest installed iOS runtime. */
async function resolveSimulatorDestination(): Promise<string> {
	const configured: string | undefined = process.env.IOS_E2E_DESTINATION;
	if (configured !== undefined) {
		return configured;
	}

	const devices = JSON.parse(
		(
			await runFile("xcrun", [
				"simctl",
				"list",
				"devices",
				"available",
				"--json",
			])
		).stdout,
	) as SimulatorDeviceList;
	const existing: SimulatorDevice | undefined = Object.values(devices.devices)
		.flat()
		.find(
			(device) =>
				device.isAvailable &&
				device.deviceTypeIdentifier === SCREENSHOT_DEVICE_TYPE,
		);
	if (existing !== undefined) {
		return `platform=iOS Simulator,id=${existing.udid}`;
	}

	const runtimes = JSON.parse(
		(
			await runFile("xcrun", [
				"simctl",
				"list",
				"runtimes",
				"available",
				"--json",
			])
		).stdout,
	) as SimulatorRuntimeList;
	const runtime: SimulatorRuntime | undefined = runtimes.runtimes
		.filter(
			(candidate) =>
				candidate.isAvailable &&
				candidate.platform === "iOS" &&
				candidate.supportedDeviceTypes.some(
					(device) => device.identifier === SCREENSHOT_DEVICE_TYPE,
				),
		)
		.sort((left, right) =>
			right.version.localeCompare(left.version, undefined, { numeric: true }),
		)[0];
	if (runtime === undefined) {
		throw new Error(
			"No installed iOS runtime supports the iPhone 16 Pro simulator",
		);
	}

	const created = await runFile("xcrun", [
		"simctl",
		"create",
		"Wallet Engine E2E iPhone 16 Pro",
		SCREENSHOT_DEVICE_TYPE,
		runtime.identifier,
	]);
	const identifier: string = created.stdout.trim();
	if (identifier.length === 0) {
		throw new Error("simctl did not return the created simulator identifier");
	}
	return `platform=iOS Simulator,id=${identifier}`;
}

/** Reserves and releases one currently unused TCP port on loopback. */
async function freePort(): Promise<number> {
	return await new Promise<number>((resolve, reject) => {
		const server = createServer();
		server.once("error", reject);
		server.listen(0, "127.0.0.1", () => {
			const address = server.address();
			if (address === null || typeof address === "string") {
				server.close();
				reject(new Error("Could not allocate a loopback port"));
				return;
			}
			server.close((error) => (error ? reject(error) : resolve(address.port)));
		});
	});
}

/** Waits for one short fixture polling interval. */
async function delay(milliseconds: number): Promise<void> {
	await new Promise<void>((resolve) => setTimeout(resolve, milliseconds));
}

/** Generates every ignored or derived artifact consumed by the iOS E2E run. */
async function prepareClientArtifacts(): Promise<void> {
	await runFile("cargo", ["xtask", "bindings", "swift"], {
		cwd: REPOSITORY_ROOT,
	});
	await runFile(
		"bun",
		[
			"--cwd",
			path.join(REPOSITORY_ROOT, "examples/web"),
			"run",
			"e2e:export-scenarios",
		],
		{ cwd: REPOSITORY_ROOT },
	);
	await runFile(
		"npm",
		[
			"--prefix",
			path.join(REPOSITORY_ROOT, "tests/ton-connect/dapp"),
			"run",
			"build",
		],
		{ cwd: REPOSITORY_ROOT },
	);
}

/** Starts shared infrastructure, runs the iOS suite, and releases every process. */
async function main(): Promise<number> {
	await prepareClientArtifacts();
	const bridge: OfficialBridge = await OfficialBridge.start();
	let dapp: DappActor | undefined;
	let provider: ProviderActor | undefined;
	try {
		dapp = await DappActor.start(bridge.url, TEST_DAPP_CONFIG);
		provider = await ProviderActor.start();
		const result: number = await runXcodebuild(bridge, dapp, provider);
		if (result !== 0) {
			process.stderr.write(`\nOfficial bridge output:\n${bridge.logs()}\n`);
			process.stderr.write(`\ndApp actor output:\n${dapp.logs()}\n`);
			process.stderr.write(`\nProvider output:\n${provider.logs()}\n`);
		}
		return result;
	} finally {
		await provider?.stop();
		await dapp?.stop();
		await bridge.stop();
	}
}

process.exitCode = await main();
