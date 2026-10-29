"use strict";var g="undefined"!=typeof globalThis?globalThis:"undefined"!=typeof window?window:"undefined"!=typeof global?global:"undefined"!=typeof self?self:{};function e(e){return e}
function freeOnUnref(f){return f}function P(){}
function playlist(t){return {intraTitlePlaylist:t.playbackUrls.intraTitlePlaylist,pauseAdsResolution:t.playbackUrls.pauseAdsResolution,other:1}}
function pause(e){return {pauseBehavior:e(e),playbackResourceExpirationTimeMs:1}}
P.prototype.resolveWithAdBreaks=freeOnUnref(function(e,t){var n=this;if(0===e.length)return;});
function source(){switch("Remote"){case"Remote":var x=this.adBreakConfigBuilder.build();return x}}
P.prototype.enqueueMediaEvents=freeOnUnref(function(e){this.applyDeferredEventTimestamp(e)});
function Q(){}
Q.prototype.enqueueMediaEvents=freeOnUnref(function(e){this.applyDeferredEventTimestamp(e)});
P.prototype.reportEvent=freeOnUnref(function(e){var t=this;this.queue.add(e),this.isQueueAtMaxDepth()});
P.prototype.recordMetric=freeOnUnref(function(e){if(null!=this.minervaClient)try{this.minervaClient.record(e)}catch(e){}});
P.prototype.uploadMinervaKpiMetrics=freeOnUnref(function(){return 1});
P.prototype.write=freeOnUnref(function(e,t){if(!(e<this.logLevel)){var n=this.toLogLine(e,t);return n}});
P.prototype.batchMessage=freeOnUnref(function(e,t){1e3<=this.batchedMessages.length});
P.prototype.send=freeOnUnref(function(e,t,n){if(void 0===t&&(t={}),this.enableEventReporting){for(var r=0;r<1;r++){}}});
P.prototype.makeAuditPingRequest=freeOnUnref(function(e,t){return e});
P.prototype.makeDirectAuditPingRequest=freeOnUnref(function(e,t){return t});
function licence(){return "untouched licence and UpdateStream PES sessions bookmarks";}
