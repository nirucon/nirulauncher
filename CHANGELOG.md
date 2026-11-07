# Changelog

## 0.4.0 (2026-11-07)

- Share view: one row per Discord webhook configured in NIRUSHARE
  (`nirushare discord list --json`, names only), *Share clipboard to Discord ·
  NAME*, or *Share image …* when the NIRUSHOT review opened the view.
- New preview view `share-discord:NAME`: it stages the content with
  `nirushare discord preview --json` and shows the target, content, size,
  number of messages and the start of the text. **Send** runs
  `nirushare discord confirm ID`, so exactly the previewed content is posted;
  **Cancel** returns to Share. Nothing is sent from the list itself.
- Needs NIRUSHARE 0.4.0 for the Discord rows; with an older NIRUSHARE they do
  not appear and everything else works as before.
- install.sh installs this release by default (it still named 0.2.45).
