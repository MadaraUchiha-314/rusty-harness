# A2UI fixtures

Samples from the A2UI specification, `specification/v0_9_1/catalogs/basic/examples/` in
<https://github.com/google/A2UI> (Apache License 2.0), copied unchanged:

| File | Source |
|---|---|
| `simple_login_form.json` | `00_simple-login-form.json` |
| `child_list_template.json` | `34_child-list-template.json` |

`renderer::a2ui::tests` parses them into the typed model and serializes them back; a
drift between the model and the published catalog shows up here first.
