function PLUGIN:BackendInstall(ctx)
	require("azure").run("install", ctx)
	return {}
end
