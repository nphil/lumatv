import QtQuick 2.4

QtObject {
    property string context: ""
    function info(id, kv, msg, force) {}
    function warning(id, kv, msg) {}
    function error(id, kv, msg) {}
    function debug(msg) {}
}
