// hello-log: Antigravity eklenti V1 örneği.
//
// Kullandığı host API'lar yalnız plugin_log ("plugin:log") ve ui_notify
// ("ui:notification"). Kancalar yükü JSON metni olarak alır; yükte belge
// içeriği, dosya yolu veya parola bulunmaz.

fn on_custom_command(payload) {
    plugin_log("hello-log: on_custom_command " + payload);
    ui_notify("Merhaba! hello-log eklentisi çalıştı.");
    "hello-log tamam"
}

fn on_document_open(payload) {
    plugin_log("hello-log: on_document_open " + payload);
}

fn on_export_complete(payload) {
    plugin_log("hello-log: on_export_complete " + payload);
}
