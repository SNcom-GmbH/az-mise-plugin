function PLUGIN:BackendListVersions(ctx)
	return require("azure").run("versions", ctx)
end
