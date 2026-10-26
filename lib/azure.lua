local cmd = require("cmd")
local json = require("json")
local M = {}

local assets = {
	["linux-amd64"] = "azure-blob-linux-amd64-musl.tar.gz",
	["windows-amd64"] = "azure-blob-windows-amd64.zip",
}

function M.run(action, ctx)
	local options = ctx.options or {}
	if not options.storage_url then
		error("azure-blob requires the storage_url tool option")
	end
	local platform = RUNTIME.osType .. "-" .. RUNTIME.archType
	local asset = assets[platform]
	if not asset then
		error("azure-blob supports only Linux and Windows x86-64; no helper build for " .. platform)
	end
	local env = {
		MISE_NO_CONFIG = "1",
		AZURE_BLOB_MISE = os.getenv("MISE_BIN") or "mise",
		-- Exact helper release and platform asset: never select a different platform.
		AZURE_BLOB_HELPER_TOOL = "github:SNcom-GmbH/az-mise-plugin[asset_pattern=" .. asset .. "]@0.5.0",
		AZURE_BLOB_REQUEST = json.encode({
			action = action,
			tool = ctx.tool,
			version = ctx.version,
			options = options,
			platform = platform,
			install_path = ctx.install_path,
			download_path = ctx.download_path,
		}),
	}
	-- Explicitly preserve this environment-only credential through cmd.exec and
	-- the nested mise exec. Never interpolate it into command text or JSON.
	local storage_token = os.getenv("AZURE_BLOB_STORAGE_TOKEN")
	if storage_token ~= nil then
		env.AZURE_BLOB_STORAGE_TOKEN = storage_token
	end
	local function execute(command)
		local ok, output = pcall(cmd.exec, command, { env = env })
		if not ok then
			-- cmd.exec includes child stderr in its error. Use plain matching so
			-- punctuation in opaque tokens cannot act as a Lua pattern.
			output = tostring(output)
			if storage_token and #storage_token > 0 then
				local start = output:find(storage_token, 1, true)
				while start do
					output = output:sub(1, start - 1) .. "[redacted]" .. output:sub(start + #storage_token)
					start = output:find(storage_token, start + #"[redacted]", true)
				end
			end
			error(output, 0)
		end
		return json.decode(output)
	end
	-- The optional local override is only for development/offline testing. All
	-- production installs use mise's verified prebuilt GitHub release backend.
	local override = os.getenv("AZURE_BLOB_HELPER")
	local command
	if override then
		env.AZURE_BLOB_HELPER = override
		command = [["$AZURE_BLOB_HELPER" hook]]
		if RUNTIME.osType == "windows" then
			-- mise already supplies cmd.exe's outer /s quote pair.
			command = [["%AZURE_BLOB_HELPER%" hook]]
		end
	else
		command = [["$AZURE_BLOB_MISE" exec "$AZURE_BLOB_HELPER_TOOL" -- azure-blob hook]]
		if RUNTIME.osType == "windows" then
			command = [["%AZURE_BLOB_MISE%" exec "%AZURE_BLOB_HELPER_TOOL%" -- azure-blob hook]]
		end
	end
	local result = execute(command)
	if result.authentication_required then
		if storage_token ~= nil then
			error("Azure storage token rejected; the caller must acquire a new token")
		end
		if os.getenv("CI") or os.getenv("AZURE_BLOB_NONINTERACTIVE") == "1" then
			error("Azure sign-in required; configure a CI identity or sign in on a Windows desktop")
		end
		env.AZURE_BLOB_LOGIN = "1"
		if cmd.stream(command, { env = env }) ~= 0 then
			error("Azure WAM sign-in failed")
		end
		env.AZURE_BLOB_LOGIN = "0"
		result = execute(command)
		if result.authentication_required then
			error("Azure still requires authentication after WAM login")
		end
	end
	return result
end

return M
