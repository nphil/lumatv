import QtQuick 2.4

// CaseMapper only upper-cases; the real one is a locale-aware JavaScript library.
QtObject {
    property var caseMapper
    function require(name) {
        return function(opts) { this.map = function(s) { return String(s).toUpperCase() } }
    }
}
