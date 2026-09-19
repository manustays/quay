// Quay agent-radar extension for pi. Installed by Quay into
// ~/.pi/agent/extensions/quay.ts; reports session state to Quay's radar by
// shelling out to the app-managed quay-hook helper (which writes the state
// file atomically). Managed by Quay — edits will be overwritten on reinstall.
//
// pi has no built-in approval/permission prompt, but `ui_prompt_start`/`ui_prompt_end`
// bracket *any* blocking prompt an extension raises (confirm/select/input/editor), and
// "blocked on a question" is exactly what the radar means by waiting.
//
// session_start  -> idle      a session exists before it has run anything
// agent_start    -> working
// ui_prompt_start-> waiting
// ui_prompt_end  -> working or idle, decided by ctx.isIdle()
// agent_settled  -> idle      not agent_end: a run can end and then be continued by
//                             pi's own retries, compaction or follow-ups, so agent_end
//                             flashed the row idle in the middle of work
// session_shutdown -> ended   clears the state file instead of leaking it
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
	pi.on("session_start", emit("idle"));
	pi.on("agent_start", emit("working"));
	pi.on("ui_prompt_start", emit("waiting"));
	// A prompt can be raised while the agent is mid-run or while it is sitting idle,
	// so the answer decides where we go back to rather than assuming work resumed.
	pi.on("ui_prompt_end", async (_event: unknown, ctx: any): Promise<void> => {
		const idle = ctx?.isIdle?.() ?? true;
		report(idle ? "idle" : "working", ctx?.sessionManager?.getSessionId?.() ?? "", ctx?.cwd ?? "");
	});
	pi.on("agent_settled", emit("idle"));
	pi.on("session_shutdown", emit("ended"));
}
