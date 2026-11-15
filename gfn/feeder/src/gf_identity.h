#ifndef GF_IDENTITY_H
#define GF_IDENTITY_H

#ifndef GF_APP_ID
#define GF_APP_ID "org.nphil.lumatv.gfn"
#endif
#ifndef GF_RUNTIME_DIR
#define GF_RUNTIME_DIR "/tmp/lumatv-gfn"
#endif
#ifndef GF_UI_PROGRAM
#define GF_UI_PROGRAM "bin64/lumatv-gfn-app"
#endif
#ifndef GF_APP_TITLE
#define GF_APP_TITLE "GeForce NOW"
#endif
#ifndef GF_UI_GAME
#define GF_UI_GAME 0
#endif

#define DEFAULT_APP_ID GF_APP_ID
#define DEFAULT_SOCKET GF_RUNTIME_DIR "/feed.sock"
#define DEFAULT_WINDOW_ID_FILE GF_RUNTIME_DIR "/window-id"
#define DEFAULT_LOG GF_RUNTIME_DIR "/feeder.log"

#endif
