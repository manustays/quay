// Quay agent-radar plugin for OpenCode. Installed by Quay into
// ~/.config/opencode/plugin/quay.js; reports session state to Quay's radar by
// shelling out to the app-managed quay-hook helper (which writes the state
// file atomically). Managed by Quay — edits will be overwritten on reinstall.
//
// OpenCode has no clean "turn started" event, so "working" comes from
// permission.replied plus Quay's own CPU heuristic; idle and waiting are exact.
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
