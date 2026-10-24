local cmd = require("cmd")
local file = require("file")
local json = require("json")
local M = {}

function M.run(action, ctx)
	local options = ctx.options or {}
	if not options.storage_url then
		error("azure-ac requires the storage_url tool option")
	end
	local env = {
		-- Ignore consumer/global/plugin mise.toml in the nested mise process.
		-- The explicit uv version is enough and needs no extra config trust.
		MISE_NO_CONFIG = "1",
		AZURE_AC_MISE = os.getenv("MISE_BIN") or "mise",
		AZURE_AC_PLUGIN = RUNTIME.pluginDirPath,
		AZURE_AC_HELPER = file.join_path(RUNTIME.pluginDirPath, "src", "azure_ac.py"),
		AZURE_AC_LAUNCHER = file.join_path(RUNTIME.pluginDirPath, "scripts", "launch.ps1"),
		AZURE_AC_REQUEST = json.encode({
			action = action,
			tool = ctx.tool,
			version = ctx.version,
			options = options,
			platform = RUNTIME.osType .. "-" .. RUNTIME.archType,
			install_path = ctx.install_path,
			download_path = ctx.download_path,
		}),
	}
	-- Nested mise runs from the plugin, not the consumer project: do not try
	-- to activate the artifact which is currently being installed.
	local command = [["$AZURE_AC_MISE" -C "$AZURE_AC_PLUGIN" exec uv@0.13.4 -- uv run --quiet --no-dev --inexact --locked --project "$AZURE_AC_PLUGIN" python "$AZURE_AC_HELPER" hook]]
	if RUNTIME.osType == "windows" then
		-- mise's default Windows inline shell is cmd. PowerShell handles the
		-- argument forwarding, so paths containing spaces remain single args.
		command = [[powershell.exe -NoProfile -NonInteractive -File "%AZURE_AC_LAUNCHER%"]]
	end
	local result = json.decode(cmd.exec(command, { env = env }))
	if result.authentication_required then
		if os.getenv("CI") or os.getenv("AZURE_AC_NONINTERACTIVE") == "1" then
			error("Azure sign-in required; authenticate on a desktop or configure CI credentials")
		end
		env.AZURE_AC_LOGIN = "1"
		if cmd.stream(command, { env = env }) ~= 0 then
			error("Azure sign-in failed")
		end
		env.AZURE_AC_LOGIN = "0"
		result = json.decode(cmd.exec(command, { env = env }))
		if result.authentication_required then
			error("Azure still requires authentication after login")
		end
	end
	return result
end

return M
