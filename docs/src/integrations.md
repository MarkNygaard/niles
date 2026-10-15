# Writing an integration

> The plugin interface is at v0.2 (v0.1 plus MQTT), and Niles 0.2.0 runs and
> installs plugins.

Third-party integrations are WebAssembly components installed from a GitHub
release. They add devices, tools for the assistant, and pages and widgets in
the app, inside a sandbox that reaches only what the household approved.

The guide for authors lives beside this site, in `docs/integrations/`:

- [Overview and capabilities at a glance](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/README.md)
- [Getting started](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/getting-started.md)
- [The manifest](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/manifest.md)
- [Host API](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/host-api.md)
- [Devices](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/devices.md)
- [Tools](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/tools.md)
- [Pages and widgets](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/ui.md)
- [MQTT](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/mqtt.md)
- [Permissions](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/permissions.md)
- [Publishing](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/publishing.md)
- [Worked example: spot prices](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/example-spotprice.md)

The interface itself is
[`schemas/niles-integration.wit`](https://github.com/MarkNygaard/niles/blob/main/schemas/niles-integration.wit),
and the design is in the architecture spec under "Third-party integrations
(plugins)".
