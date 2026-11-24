"use strict";var g="undefined"!=typeof globalThis?globalThis:"undefined"!=typeof window?window:"undefined"!=typeof global?global:"undefined"!=typeof self?self:{};function e(e){return e}
function freeOnUnref(f){return f}function P(){}
function playlist(t){return {intraTitlePlaylist:t.playbackUrls.intraTitlePlaylist,pauseAdsResolution:t.playbackUrls.pauseAdsResolution,other:1}}
function pause(e){return {pauseBehavior:e(e),playbackResourceExpirationTimeMs:1}}
function pauseNew(){return {pauseBehavior:function(e){var t=K.PlaybackResourceUtils.valueOrUndefined(e.playbackUrls);if(null==t){var n=K.PlaybackResourceUtils.valueOrUndefined(e.playlistedPlaybackUrls);if(null!=n)return n.pauseAdsResolution}}}}
P.prototype.resolveWithAdBreaks=freeOnUnref(function(e,t){var n=this;if(0===e.length)return;});
function source(){switch("Remote"){case"Remote":var x=this.adBreakConfigBuilder.build();return x}}
function live(e,t){return null!=e.showLivePreRolls&&!0===e.showLivePreRolls.enabled&&!isFinite(t)}
function liveDeep(){return null!=cg.store.cfg.showLivePreRolls&&!0===cg.store.cfg.showLivePreRolls.enabled}
function liveAgain(){return null!=cg.store.cfg.showLivePreRolls&&!0===cg.store.cfg.showLivePreRolls.enabled&&"LiveStreaming"===this.type}
P.prototype.enqueueMediaEvents=freeOnUnref(function(e){this.mediaEventBatcher.batchMediaEvents(e)&&this.dispatchMediaEvents();for(var t=0,n=e;t<n.length;t++){var r=n[t];this.telemetryReporterDelegator.reportEvent(r)}});
function Q(){}
Q.prototype.enqueueMediaEvents=freeOnUnref(function(e){for(var t=0,n=e;t<n.length;t++){var r=n[t];this.platformReporter.reportEvent(r)}});
function R(){}
R.prototype.enqueueMediaEvents=freeOnUnref(function(e){this.applyDeferredEventTimestamp(e)});
P.prototype.reportEvent=freeOnUnref(function(e){var t=this;this.queue.add(e),this.isQueueAtMaxDepth()});
P.prototype.recordMetric=freeOnUnref(function(e){if(null!=this.minervaClient)try{this.minervaClient.record(e)}catch(e){}});
P.prototype.uploadMinervaKpiMetrics=freeOnUnref(function(){return 1});
P.prototype.write=freeOnUnref(function(e,t){if(!(e<this.logLevel)){var n=this.toLogLine(e,t);return n}});
P.prototype.batchMessage=freeOnUnref(function(e,t){1e3<=this.batchedMessages.length});
P.prototype.send=freeOnUnref(function(e,t,n){if(void 0===t&&(t={}),this.enableEventReporting){for(var r=0;r<1;r++){}}});
P.prototype.makeAuditPingRequest=freeOnUnref(function(e,t){return e});
P.prototype.makeDirectAuditPingRequest=freeOnUnref(function(e,t){return t});
var beacons=[{key:"dispatch",value:function(o,t){var a,s=this,l=o.url,u="UnknownError",f=0;return a}}];
function licence(){return "untouched licence and UpdateStream PES sessions bookmarks";}
