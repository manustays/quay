// Quay agent-radar extension for pi. Installed by Quay into
// ~/.pi/agent/extensions/quay.ts; reports session state to Quay's radar by
// shelling out to the app-managed quay-hook helper (which writes the state
// file atomically). Managed by Quay — edits will be overwritten on reinstall.
//
// pi has no built-in approval/permission prompt, so it reports working/idle only.
// agent_start -> working, agent_settled -> idle.
//
// `agent_settled`, not `agent_end`: an agent run can end and then be continued by
// pi's own automatic retries, compaction or follow-ups, so `agent_end` flashed the
// row idle in the middle of work. `agent_settled` fires once all of that is done.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";

const HOOK = path.join(os.homedir(), "Library/Application Support/am.abhi.quay/bin/quay-hook");

function report(state: string, sessionId: string, cwd: string): void {
	try {
		const id = sessionId || "pi-" + Buffer.from(cwd || "").toString("hex").slice(0, 16);
		const child = spawn(HOOK, [state, "pi"], { stdio: ["pipe", "ignore", "ignore"] });
		child.on("error", () => {});
		child.stdin.on("error", () => {});
		child.stdin.write(JSON.stringify({ session_id: id, cwd: cwd || "" }));
		child.stdin.end();
	} catch {}
}

export default function (pi: ExtensionAPI): void {
	const emit = (state: string) => async (_event: unknown, ctx: any): Promise<void> => {
		report(state, ctx?.sessionManager?.getSessionId?.() ?? "", ctx?.cwd ?? "");
	};
	pi.on("agent_start", emit("working"));
	pi.on("agent_settled", emit("idle"));
}
