{
  "targets": [
    {
      "target_name": "mismatched_abi_version",
      "sources": ["mismatched_abi_version.cpp"],
    },
    {
      "target_name": "no_entrypoint",
      "sources": ["no_entrypoint.cpp"],
    },
    {
      "target_name": "initializer_only",
      "sources": ["initializer_only.cpp"],
    }
  ],
  "conditions": [
    # It imports zlib from the host, which opens only with lazy binding, and that is not the default of every compiler.
    ["OS=='linux'", {
      "targets": [
        {
          "target_name": "uws_shaped",
          "sources": ["uws_shaped.cpp"],
          "ldflags": ["-Wl,-z,lazy"],
        }
      ],
    }],
  ]
}
