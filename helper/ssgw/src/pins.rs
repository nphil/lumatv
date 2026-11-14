//! The LG files whose behaviour this gateway reproduces, by SHA-256, as on webOS 10.2.1 (`Rockhopper release 10.2.1-5202`). A
//! firmware that changes any of them, or adds an interface or permission file, stops the gateway (exit 78) so the set stays on
//! LG's until these are checked again.

use std::path::Path;

pub const PINNED: &[(&str, &str)] = &[
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/api.interface", "c69071fcc0823b503eb240badd9843f1fb6fd2c6ba79c6bd342b2cccaea2800d"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/audio.interface", "b3174570f63a9dc0708ba3aa06811e3513ce82d152e78bcb6a33b6d7dea5b33b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.palm.systemservice.interface", "60ec27e3dd7b702d0e3e58faccadba097e0be36a21bf527a36a3cf3fb0afe854"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.applicationManager.interface", "92629822de811d152415c429bc3a204b10feabb1e27c6facd4709dfd495fe281"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.launcher.interface", "3ab9e01e72a63f1574e873ca3073cca987e162eea638113cae5d8be2d18c3cb1"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.media.interface", "234593be8ca5b87bbf78abbdc96d78eff7b31144d3e4795e7238fd3e30113340"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.notification.interface", "be2571925f17698add185cb3313ef0e2090d5ba259c5305eef5680db7ad88ecb"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.apiadapter.interface", "66503f568bae76cf31741525bea7524766b2fde26ee811a65d519638aabe0b59"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.apiadapter.pub.interface", "a615ad94ee1ce156397b6322eca8f6b502228c3537023b73500354088b452a79"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.apluspolicymanager.interface", "17d26804a45372e75c3452d1b9844b4396e6ef8612b5ba977e5d71aef31e2fac"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.appcasting.interface", "a30e2b41f1cc1c3c3a3eb40c86457606db1bce256940fe5a5f540599b35d44c5"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.asc.interface", "c76441233e4d8bc7374503863941381c5283e4d5b84b0284862f95a99ec0a26d"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.attachedstoragemanager.interface", "9141e3295509f60276f46a3626d199ea5932bc66482d52f7c5574002c798c753"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.audio.interface", "1f611f8149993e67e5c769115ad3189140bad80325182ec57995b05565f8a385"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.bluetooth.interface", "4be364ccc24eb0574d5782be1ada490652aa610741d7023b4356b24c77feb25e"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.bluetooth2.interface", "e20c548cee548ba570339cdd4057ebed07928397fb01b684834087d21ab36e4b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.bosp.interface", "965f71a37a77bdbfe870c97663c82e318e9627c745a344a12eb1f56463e4b8a7"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.btaudiosnk.interface", "7ed27f647c8d139b449d86327f74ebf677c46246a65e62708cd6d1d8620101d6"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.btaudiosrc.interface", "f28a9b211ca524cb06481869b6b90d0a71e7eb0150a83d0425c1b78401fdaa4c"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.capture.interface", "f9d9957a0675903b40a73e2518280d08da38132fc79b74031e9679638badcc17"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.cis.interface", "a2fda74ab0ffc5f18200745ea9964f5ca28715368fd664d3684a7703d141e72c"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.connectionmanager.interface", "30993ff78e2796c80dfa9b29b5cb7337180021c423f187381953c2d9af12e325"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.devicereset.interface", "6e9597ce412d650844af7afbf1ec28341020c59c70d12a9fb8b4e2298670675d"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.downloadmanager.interface", "43bf6dc2fdecab7253a787201aa611e8671679e629f60c4fb8a18e7414297f0f"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.eim.interface", "c4a958a9525d418e00b76bc81af38933bc1c44480261791ad6c21675459bf75b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.firstuse.interface", "6684b65e3d83fd058b85852f5ba50a3bd48ae41a40c37eeff6253af68db382ff"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.homelaunchpoints.interface", "142728dd284ea436438f06c93d4a7aae2e72586579d8496b7fa659f3d2a15363"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.ics.interface", "fe9d4184e739e5cd6e0e51074d32c2dca1d9d3bf0dd5ea09da91ef043d3c5777"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.iepg.interface", "5ac28412e55e1e8ea4b69c8c8421095aee43f688e495569a28744e44fcd03e75"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.ime.interface", "cf2caa3e5a9f818dadd9dbdb8768af5ab1dffb921fcaee1622d7459db4ea374b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.iotclient.interface", "1cb3c439a51b76f4f5f479299b5b587164560ec557d045ad1fa10aa0d0fd1c83"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.irdbmanager.interface", "da10581752966648f91444d7f6f13e5a0a7244c37f8c9bfcea4ad8843017bd89"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.livepick.interface", "b96c6f68a9af9450a1aabca065ce0dc7e7abb372a04169b3c10a4975279393aa"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.miracast.interface", "1a6feddb2427c5357334126b119c90d1a494732a4c49ba4318db916b543c49ae"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.miracasttx.interface", "2e687674196f344e53c8b728558f606bbb123a458b2a0cbdf921e8113588a583"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.mrcu.interface", "f527296b274e624408a95136ecec38b127830c8e7c008cf1d4bc80f96e28f277"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.networkinput.interface", "5c109af0137fca612fb7cf84b9daed3a4f9eebbab80d48441952025709f0b9dd"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.nlpmanager.interface", "bf87638e0bc0caea7e8d4d023a32f09a2b30d578c63757420a7e9f6fe56e8ca8"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.panelcontroller.interface", "39a5144063969d93ff8099d4f8d9663bcfa020d9050f562877e7974a72764128"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.pushclient.interface", "d9bff96b6ac08c51c16a49e385aa29bd4807ff104188168ebdcbf1a1dfe558d0"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.reclistmgr.list.interface", "31c079199c89b4d269a83894a7c415c5ba4b68fa295ce70148b0e85e98a2e499"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.rollingscreen.interface", "8b163cc4d4087db8f22ec27c606a8ed1dbcc6763ca574cecbbe0207ec942536e"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.sdx.interface", "785bcf1acfdd8c71e0985b8ff75e970ffb4cfc344e574e99666425f0c827bfcd"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.secondscreen.gateway.interface", "e59ed2c789734f3f6c2f8273a2e5a9d640bc6c6e4c5b0d27c191cc632866c558"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.sm.interface", "008df7483ab75809cf525cf0e67822a98514d49b85a8ac722e1e4ce3705b973b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.soundshare.interface", "760c5d8d2bc1c3da6e62a0bd56ffece7f2184927145f41b72bb3fe71ffdf078f"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.systemprofile.interface", "2972c0ad862ed8045cac1f943c318855bdc113a04acb72d1a4bd1b97608c961c"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.tvReservationAgent.interface", "3135113def4a6b8a3b6b3400458e548b24d9950f2f3ebff725cd2d7a774eb2fe"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.tvpower.interface", "ab1b1cb9b73b8452f56281b415b8fc6b56b1600f4a5ee5dd189099c1ddef5313"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.update.interface", "a4cb88cb61084af530800ef9ab9dea8ee17605eb510a30cc866f10dbd1ead651"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.utp.channel.interface", "bac6fa15ada470f84b078f91411873758aa7ef6c248aef17014eebe5cf6da58c"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.utp.dvr.interface", "7d9a31c9d83228f190ddb6ab026cbe1477d878768a0fcda7f34c173741ad88b6"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.voiceconductor.interface", "8ba8203a0dd17a12f4d6ffb560e08829dccf3e2a3f6eb054a1b7341fab3c86a4"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.voiceinput.interface", "479e67893238dc5d6705146c0840ede21ef2343faa4a152f17cf2761c5e31fd6"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.wifi.interface", "6df232a4ec43e4aa625dfd81d13935c5aa0ec55c2da9e5469a4fe9dbea0bf51c"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.service.wirelessavcontroller.interface", "989a430777a69517d03616222d8d817a8afd6d5a5f4f5132bcc4a3e7f2c67de7"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.surfacemanager.input.interface", "4638f753c76a410f8970165680d1225c6b7450cfe46a8d71a5fdb5e7110d4d89"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.surfacemanager.interface", "5be455cb46f31a17e1ae67bba3136447103ef7ff33d004dab226d1e553ead6fd"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/com.webos.universalsearch.interface", "d2c11bf14ee844ae1e5f0a5f5be40fc1194f8e888efd4213d1090649df0a1335"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/config.interface", "d3648f99a813d458dfdfed298ea405f8f344a93d9bd1a196c26b3129dfe27980"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/externalpq.interface", "217a9d4755b590d494d43a02003d0e4533e513721152e21d27e3c80e9aac40f9"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/media.controls.interface", "eecaedfcdea8aebd643a5502749ba6f8e06af04347ad45d863de5fed0cba15fe"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/media.viewer.interface", "787350e7b1fc4e27b7022f9046961703c8368d3fc147791a3a09a4527e6c6581"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/pairing.interface", "0910cf1a97866d9a35c97178bd2059ef627dc384014541647e7802c51a072aa8"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/settings.interface", "7e5f7074eeae6e718b54ccc1c631be46139ec5cc3f949930411ffd3e0efd8d3f"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/system.interface", "e2d0aea186cc5a5e907fa6e3dc58bb2edd42861ce426b73f434ccb7223fd7b32"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/system.launcher.interface", "27ec0152c82089ddd082effa80010216def94e5ca335307f1cdac91d03615790"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/system.notifications.interface", "6d1ee7c26b3c974dfecad2779bffcdade3cd246cfad0c4359d7e1fcbc6979c7e"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/timer.interface", "a6df8c64b320a892336926cab20ae1ac7ec7fdc4334af49b8204dc6678bd8332"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/tv.interface", "8f9a5d6ba184acd4bbf291dbddb0321cc412f78ac8e01b712e6638051adcc449"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/user.interface", "73aa650f0462cede71047e0e9cdb60048eccfbfedd9f113d0addadb6c0573956"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces/webapp.interface", "0f7377a314e1e7076f499c8deb530c2c892e2c117be279c252f394573781b5f7"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/permission-types/dvr.permissions", "6dcd14e355679417b5c4ec72a7bd978164c44b69cf4d53e7ba62b84267c164de"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/permission-types/lge.permissions", "371a486a55bb1161d3036f837c444e6efbd754e03f98306d254b7d2ce02c921b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/permission-types/system.permissions", "1a4743db665dda822bc07ae03849c362a57f81e47b4232a1fd4f0398abe1ae67"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/permission-types/test.permissions", "8a3c16c7d61ca3d18f0249d8117ef93feba4b6cd35652f5c10e5bf055aeb1f24"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/permission-types/tv.permissions", "9b58f34191ff5f82c478decb0853f8ba8f154f01cd298c9ebc73089f435358d4"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/config.js", "d428b1731725ccb989792168710eec08efe54d424ea9149e6988f6a7ec12c012"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/adapters/misc/index.js", "921cba03e1c1d47c575b59ba037e44380aa7576865ce47f36688132ef3b3f262"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/adapters/pairing/index.js", "6114e52c42eaca144590fbea525e4cecbcada15cc9f033cde74515fd4a14e238"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/servers/http-server.js", "5fce2cbd93626148796d08affbc8283fd99b0e2537457d100a54c7d9de1f69f9"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/servers/resource-server.js", "20a817ae4431b3a825094805b23341120fe4657640afc28d733d998404aae052"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/servers/web-socket-client-manager.js", "f937c4ef17a521467409715a3cb9d6d13308faedfa48618ed56e879b3851b5eb"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/servers/web-socket-proxy.js", "1cb04f41fb496b42cf438bbc56590180eb1d8d5da81060584503149a45e2afcb"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/servers/web-socket-remote-client.js", "42df610bb9a52de7b91985d7453c1cbc8cea05a3975635585007cdae420011dd"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/servers/web-socket-utils.js", "8ac026c2c83e662d8c8f37fb7be7af2c6301a0478d16e15f1dc0906264dd0f3f"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/system/gateway.js", "cbda720d99768f76df8c4c61aad1f095536b153752660d05a46278ccbc4c9827"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/system/interface-manager.js", "9fc020beb09f8b0c89882a2c9cfb1b89c97d8fe15deb6fd2edca8bbf74858eb2"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/system/interface/interface-description.js", "e972fc613a7c735be497f084eacfc9ee540a6b2f3e46ceaf5344a8817ebadb5a"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/system/interface/method-description.js", "cb4882c9d555c6ab7caafc47183a30f233d088752721d005b3f4dbbb6e7d29c5"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/system/permissions-manager.js", "b53785c426650db57614cf6f28e4106140e245acb6e99391f6c9dab5e10b3c25"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/system/power-manager.js", "f74521991993a8e33ed7477f0ef88a1eeeafbf8c0b162e6aad143b754b94029d"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/bus/settings-manager.js", "06ae732192c70a1039eff714b25082905391d5b8541a1981f4914e7f98516cf9"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/bus/shared-client.js", "6d1993d1bd83acab918658f7ba6d6c1a00697c1206e0c534ec2368589edb212b"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/bus/filtering-uris.js", "68c3698dfebd0c3671993e06c681b3f5ade4aa26cbff2072281fd0ddc9387576"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/bus/uri-filter.js", "27a59f2be1b146e2355d1559996b98476b42cae2f852a1dbdc61617f8eff4aff"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/bus/service/client-profile.js", "b4a450e066e32fde8bf8f305b438a8f3fedebefa3581a4669d9ee6ce03d25566"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/helpers/bus/service/client-profile-manager.js", "e1234e2e2170aae20abd2a2bc2e51b784f777f2c868f5f89ed90e22db07082ab"),
    ("usr/palm/services/com.webos.service.apiadapter/adapters/audio/index.js", "b8c1135297031190f85213cbc27808ffc67d2b12bfbcdae368a7be38956dee4c"),
    ("usr/palm/services/com.webos.service.apiadapter/adapters/system/index.js", "d8a7104d66ff52dd27b2f6309073ed0acd80d8e64a0e938070cc6780bbc71565"),
    ("usr/palm/services/com.webos.service.apiadapter/adapters/system_launcher/index.js", "c2b09d1f51deab111ee859dd45faedd64b549847c5bc0f5f3b80ec930d895cc3"),
    ("usr/palm/services/com.webos.service.apiadapter/adapters/system_notifications/index.js", "8b5f1738ffbcb6dd54a924c6194f32c4047b9c5e2c40445a13f4115389f4c8b3"),
    ("usr/palm/services/com.webos.service.apiadapter/adapters/tv/index.js", "e8e30c9f49b076814a4da6202ac8984224b884653d34897b88622e03746c5fbe"),
    ("usr/palm/services/com.webos.service.apiadapter/adapters/tv/channel-utils.js", "5ac93e3936f2a3d389dd6d9f8724af44bb2af4133b61c4b60de5ee27cb792b43"),
    ("usr/palm/services/com.webos.service.apiadapter/helpers/adapter/request.js", "7d4bd0f609fd1ae6d6718364c450597a6d4e52c035a8ec0f1a6c93185e56fe82"),
    ("usr/palm/services/com.webos.service.apiadapter/helpers/system/launch-manager.js", "65868c99ffdaccb491aa1f8ebc5541ac186ccaf019132fc2fcdb37a56f992e21"),
    ("usr/palm/services/com.webos.service.apiadapter/helpers/system/file-cache-manager.js", "e966f924c0dcda66b54bc095f420768ac356f45bed21f5a0b79e3b2cf5cdfac5"),
    ("usr/palm/services/com.webos.service.apiadapter.pub/adapters/application_manager/index.js", "0530023cd4f6d1fe3619fe24671e299a416810456b9d40d15587b45061c7abc5"),
];

/// The directories whose every file is pinned: a file in them that is not in [`PINNED`] is a change too.
const CLOSED_DIRS: &[(&str, &str)] = &[
    ("usr/palm/services/com.webos.service.secondscreen.gateway/interfaces", ".interface"),
    ("usr/palm/services/com.webos.service.secondscreen.gateway/permission-types", ".permissions"),
];

/// Checks the files under `root` against `pins`; the error names the first file that differs.
pub fn check(root: &Path, pins: &[(&str, &str)]) -> Result<(), String> {
    for (rel, want) in pins {
        let path = root.join(rel);
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if crate::util::sha256_hex(&data) != *want {
            return Err(format!("{} differs from the version this gateway reproduces", path.display()));
        }
    }
    for (dir, suffix) in CLOSED_DIRS {
        if !pins.iter().any(|(p, _)| p.starts_with(dir)) {
            continue;
        }
        let path = root.join(dir);
        for entry in std::fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(suffix) && !pins.iter().any(|(p, _)| p.strip_prefix(dir).and_then(|r| r.strip_prefix('/')) == Some(&*name)) {
                return Err(format!("{}/{name} is new", path.display()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_missing_and_added_files_are_named() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(CLOSED_DIRS[0].0);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.interface"), b"{}").unwrap();
        let rel = format!("{}/a.interface", CLOSED_DIRS[0].0);
        let good = [(rel.as_str(), "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a")];
        assert_eq!(check(root.path(), &good), Ok(()));
        std::fs::write(dir.join("b.interface"), b"{}").unwrap();
        assert!(check(root.path(), &good).unwrap_err().ends_with("b.interface is new"));
        std::fs::remove_file(dir.join("b.interface")).unwrap();
        std::fs::write(dir.join("a.interface"), b"{ }").unwrap();
        assert!(check(root.path(), &good).unwrap_err().contains("a.interface differs"));
        std::fs::remove_file(dir.join("a.interface")).unwrap();
        assert!(check(root.path(), &good).is_err());
    }
}
