// Quay agent-radar plugin for OpenCode. Installed by Quay into
// ~/.config/opencode/plugin/quay.js; reports session state to Quay's radar by
// shelling out to the app-managed quay-hook helper (which writes the state
// file atomically). Managed by Quay — edits will be overwritten on reinstall.
//
// session.created      -> idle     a session exists before it has run anything
// tool.execute.before   -> working
// permission.asked      -> waiting
// permission.replied    -> working
// session.idle          -> idle
// session.deleted       -> ended
//
// `tool.execute.before` is the closest thing OpenCode has to "turn started": it is
// bounded (once per tool call, like Claude's PostToolUse) where message.updated fires
// on every stream chunk and would spawn the helper continuously. It does not cover a
// turn that only thinks and never calls a tool, so a short reply can still read idle
// until session.idle confirms it — narrower than the old CPU heuristic, not a total
// replacement for it.
import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";

const HOOK = path.join(os.homedir(), "Library/Application Support/am.abhi.quay/bin/quay-hook");

function report(state, sessionID, cwd) {
	try {
		// Deterministic per-cwd fallback id (hex is safe_id-clean) so distinct
		// project folders keep distinct state files even when sessionID is absent.
		const id = sessionID
			? String(sessionID)
			: "oc-" + Buffer.from(cwd || "").toString("hex").slice(0, 16);
		const child = spawn(HOOK, [state, "opencode"], { stdio: ["pipe", "ignore", "ignore"] });
		child.on("error", () => {});
		child.stdin.on("error", () => {});
		child.stdin.write(JSON.stringify({ session_id: id, cwd: cwd || "" }));
		child.stdin.end();
	} catch {}
}

export const QuayRadar = async ({ directory }) => ({
	event: async ({ event }) => {
		const id = event?.properties?.sessionID;
		switch (event?.type) {
			case "session.created":
				return report("idle", id, directory);
			case "tool.execute.before":
				return report("working", id, directory);
			case "session.idle":
				return report("idle", id, directory);
			case "permission.asked":
				return report("waiting", id, directory);
			case "permission.replied":
				return report("working", id, directory);
			case "session.deleted":
				return report("ended", id, directory);
		}
	},
});
