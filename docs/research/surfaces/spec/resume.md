State on 2026-10-23: the catalogue is complete. All 160 app ids and all 113
compositor view files are in a row or in "No UI" (script check). Raw rows with
evidence for every slice are in `docs/research/surfaces/*.json`; the English
service message strings are in `service-strings.txt` there. The TV was only
read: no setting, app or power change, left in standby.

To refresh after a firmware update: pull the small text files of
`/usr/palm/applications` and the QML modules from the set (paramiko, stream
`find ... | tar -T - -cf -`; BusyBox tar has no `--null`), diff the app ids
against `listApps`, and compare `configd` `systemUIManager` and `keyFilters`.
