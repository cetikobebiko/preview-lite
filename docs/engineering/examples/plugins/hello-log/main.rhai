// hello-log: Antigravity eklenti V1 örneği.
//
// Kullandığı host API'lar yalnız plugin_log ("plugin:log") ve ui_notify
// ("ui:notification"). Her kanca, şeması doğrulanmış yükü (payload V1) bir nesne
// haritası olarak alır (sayıları metne çevirmek için .to_string()):
//   on_document_open   #{ schema_version: 1, page_count: N }
//   on_export_complete #{ schema_version: 1, format: "docx", page_count: N }
//   on_custom_command  #{ schema_version: 1 }
// Yükte belge içeriği, dosya adı, dosya yolu veya parola bulunmaz.

fn on_custom_command(payload) {
    plugin_log("hello-log: on_custom_command schema_version=" + payload.schema_version.to_string());
    ui_notify("Merhaba! hello-log eklentisi çalıştı.");
    "hello-log tamam"
}

fn on_document_open(payload) {
    plugin_log("hello-log: on_document_open page_count=" + payload.page_count.to_string());
    ui_notify("hello-log: belge açıldı, " + payload.page_count.to_string() + " sayfa.");
}

fn on_export_complete(payload) {
    plugin_log("hello-log: on_export_complete format=" + payload.format + " page_count=" + payload.page_count.to_string());
    ui_notify("hello-log: dışa aktarma tamamlandı (" + payload.format + ", " + payload.page_count.to_string() + " sayfa).");
}
