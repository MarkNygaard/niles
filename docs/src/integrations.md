# Writing an add-on

> The plugin interface is `niles:integration@0.3.0`: version 0.2.1 plus
> stored secrets, account sign-in (`authorize-url`, `on-callback`) and
> webhooks (`on-webhook`). Add-ons built for 0.1 and 0.2 still run. Niles
> 0.3.0 runs and installs plugins.

Third-party add-ons are WebAssembly components installed from a GitHub
release. They add devices, tools for the assistant, and pages and widgets in
the app, inside a sandbox that reaches only what the household approved.
For how a household installs and manages one, see
[Add-ons and accounts](add-ons.md#add-ons-from-github).

The guide for authors lives beside this site, in `docs/integrations/`:

- [Overview and capabilities at a glance](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/README.md)
- [Getting started](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/getting-started.md)
- [The manifest](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/manifest.md)
- [Host API](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/host-api.md)
- [Devices](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/devices.md)
- [Tools](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/tools.md)
- [Pages and widgets](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/ui.md)
- [MQTT](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/mqtt.md)
- [Accounts and webhooks](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/accounts.md)
- [Permissions](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/permissions.md)
- [Publishing](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/publishing.md)
- [Worked example: spot prices](https://github.com/MarkNygaard/niles/blob/main/docs/integrations/example-spotprice.md)

The interface itself is
[`schemas/niles-integration.wit`](https://github.com/MarkNygaard/niles/blob/main/schemas/niles-integration.wit),
and the design is in the architecture spec under "Third-party add-ons
(plugins)".
