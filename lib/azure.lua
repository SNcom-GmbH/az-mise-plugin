local cmd = require("cmd")
local json = require("json")
local M = {}

local assets = {
	["linux-amd64"] = "azure-ac-linux-amd64-musl.tar.gz",
	["linux-arm64"] = "azure-ac-linux-arm64-musl.tar.gz",
	["windows-amd64"] = "azure-ac-windows-amd64.zip",
}

function M.run(action, ctx)
	local options = ctx.options or {}
	if not options.storage_url then
		error("azure-ac requires the storage_url tool option")
	end
	local platform = RUNTIME.osType .. "-" .. RUNTIME.archType
	local asset = assets[platform]
	if not asset then
		error("azure-ac has no helper build for " .. platform)
	end
	local env = {
		MISE_NO_CONFIG = "1",
		AZURE_AC_MISE = os.getenv("MISE_BIN") or "mise",
		-- Exact helper release and platform asset: never select a different platform.
		AZURE_AC_HELPER_TOOL = "github:DominicWrege/az-mise-plugin[asset_pattern=" .. asset .. "]@0.2.0",
		AZURE_AC_REQUEST = json.encode({
			action = action,
			tool = ctx.tool,
			version = ctx.version,
			options = options,
			platform = platform,
			install_path = ctx.install_path,
			download_path = ctx.download_path,
		}),
	}
	-- The optional local override is only for development/offline testing. All
	-- production installs use mise's verified prebuilt GitHub release backend.
	local override = os.getenv("AZURE_AC_HELPER")
	local command
	if override then
		env.AZURE_AC_HELPER = override
		command = [["$AZURE_AC_HELPER" hook]]
		if RUNTIME.osType == "windows" then
			command = [[""%AZURE_AC_HELPER%" hook"]]
		end
	else
		command = [["$AZURE_AC_MISE" exec "$AZURE_AC_HELPER_TOOL" -- azure-ac hook]]
		if RUNTIME.osType == "windows" then
			command = [[""%AZURE_AC_MISE%" exec "%AZURE_AC_HELPER_TOOL%" -- azure-ac hook"]]
		end
	end
	local result = json.decode(cmd.exec(command, { env = env }))
	if result.authentication_required then
		if os.getenv("CI") or os.getenv("AZURE_AC_NONINTERACTIVE") == "1" then
			error("Azure sign-in required; configure a CI identity or sign in on a Windows desktop")
		end
		env.AZURE_AC_LOGIN = "1"
		if cmd.stream(command, { env = env }) ~= 0 then
			error("Azure WAM sign-in failed")
		end
		env.AZURE_AC_LOGIN = "0"
		result = json.decode(cmd.exec(command, { env = env }))
		if result.authentication_required then
			error("Azure still requires authentication after WAM login")
		end
	end
	return result
end

return M
