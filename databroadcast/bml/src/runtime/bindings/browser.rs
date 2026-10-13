// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `browser` オブジェクト、`Ureg`・`Greg`、決まった値を返す小さな関数、数の書式。

use super::*;

pub(super) fn browser_object(context: &mut Context) -> JsObject {
    let ureg = register_array(context, true);
    let greg = register_array(context, false);
    let mut object = ObjectInitializer::new(context);
    object.property(js_string!("Ureg"), ureg, Attribute::all());
    object.property(js_string!("Greg"), greg, Attribute::all());
    for (name, f, length) in [
        (
            "getBrowserVersion",
            browser_version as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            0,
        ),
        ("getBrowserSupport", browser_support, 1),
        ("getBrowserStatus", browser_status, 3),
        ("getResidentAppVersion", resident_app_version, 1),
        ("getIRDID", ird_id, 1),
        ("launchDocument", launch_document, 1),
        ("launchDocumentRestricted", launch_document, 1),
        ("reloadActiveDocument", reload_document, 0),
        ("quitDocument", quit_document, 0),
        ("getActiveDocument", active_document, 0),
        ("lockModuleOnMemory", lock_module, 1),
        ("lockModuleOnMemoryEx", lock_module_ex, 1),
        ("unlockModuleOnMemory", unlock_module, 1),
        ("unlockModuleOnMemoryEx", unlock_module, 1),
        ("unlockAllModulesOnMemory", unlock_all_modules, 0),
        ("getLockedModuleInfo", locked_module_info, 0),
        ("isBeingBroadcast", is_being_broadcast, 1),
        ("getFreeContentsMemory", free_memory, 0),
        ("setInterval", set_interval, 3),
        ("setTimeout", set_timeout, 2),
        ("clearTimer", clear_timer, 1),
        ("pauseTimer", pause_timer, 1),
        ("resumeTimer", resume_timer, 1),
        ("sleep", sleep, 1),
        ("random", random, 1),
        ("getProgramID", program_id, 1),
        ("epgGetEventStartTime", event_start_time, 1),
        ("epgGetEventDuration", event_duration, 1),
        ("epgTune", epg_tune, 1),
        ("epgTuneToComponent", epg_tune, 1),
        ("epgIsReserved", epg_is_reserved, 1),
        ("getTuningLinkageSource", tuning_linkage_source, 0),
        ("getTuningLinkageType", tuning_linkage_type, 0),
        ("readPersistentArray", read_persistent, 2),
        ("writePersistentArray", write_persistent, 3),
        ("readPersistentArrayWithAccessCheck", read_persistent, 2),
        ("writePersistentArrayWithAccessCheck", write_persistent, 3),
        ("checkAccessInfoOfPersistentArray", check_access_info, 1),
        ("setCurrentDateMode", set_date_mode, 1),
        ("subDate", sub_date, 3),
        ("addDate", add_date, 3),
        ("formatNumber", format_number, 1),
        ("playRomSound", play_rom_sound, 1),
        ("lockScreen", ok_one, 0),
        ("unlockScreen", ok_one, 0),
        ("loadDRCS", load_drcs, 1),
        ("unloadDRCS", unload_drcs, 0),
        ("getNPT", get_npt, 0),
        ("getProgramRelativeTime", program_relative_time, 0),
        ("isIPConnected", is_ip_connected, 0),
        ("getConnectionType", connection_type, 0),
        ("transmitTextDataOverIP", transmit_text_data, 3),
        ("confirmIPNetwork", confirm_ip_network, 3),
        ("setCacheResourceOverIP", nan_fn, 1),
        ("detectComponent", detect_component, 1),
        ("isSupportedMedia", is_supported_media, 1),
        ("getCCDisplayStatus", cc_display_status, 1),
        ("setCCDisplayStatus", set_cc_display_status, 2),
        ("getCCLanguageStatus", cc_language_status, 1),
        ("startResidentApp", nan_fn, 3),
        ("X_DPA_startResidentApp", nan_fn, 3),
        ("X_DPA_getComBrowserUA", com_browser_ua, 0),
        ("X_DPA_launchDocWithLink", nan_fn, 1),
        ("X_DPA_getIRDID", ird_id, 1),
        ("X_DPA_writeCproBM", nan_fn, 3),
        ("X_CSP_setAccessInfoToProviderArea", nan_fn, 2),
        ("launchExApp", nan_fn, 2),
        ("readBookmarkArray", null_fn, 2),
        ("writeBookmarkArray", nan_fn, 3),
        ("getBookmarkInfo", null_fn, 0),
        ("isRootCertificateExisting", is_root_certificate_existing, 2),
        ("getRootCertificateInfo", root_certificate_info, 0),
        ("vote", nan_fn, 1),
        ("startAITControlledApp", start_ait_controlled_app, 3),
        ("startReceivingAIT", start_receiving_ait, 1),
        ("getReceivedAIT", received_ait, 1),
        ("stopReceivingAIT", stop_receiving_ait, 0),
        ("requestAnimationFrame", nan_fn, 1),
        // TR-B14 第一分冊 表 5-10 の残り。規格の「できない」を返す。
        ("epgReserve", nan_fn, 1),
        ("epgCancelReservation", nan_fn, 1),
        ("epgRecIsReserved", nan_fn, 1),
        ("epgRecReserve", nan_fn, 1),
        ("epgRecCancelReservation", nan_fn, 1),
        ("seriesIsReserved", nan_fn, 1),
        ("seriesReserve", nan_fn, 1),
        ("seriesCancelReservation", nan_fn, 1),
        ("seriesRecIsReserved", nan_fn, 1),
        ("seriesRecReserve", nan_fn, 1),
        ("seriesRecCancelReservation", nan_fn, 1),
        ("connect", nan_fn, 4),
        ("disconnect", nan_fn, 0),
        ("sendTextData", nan_fn, 2),
        ("receiveTextData", null_fn, 1),
        ("setISPParams", nan_fn, 1),
        ("getISPParams", null_fn, 0),
        ("connectPPP", nan_fn, 2),
        ("connectPPPWithISPParams", nan_fn, 1),
        ("disconnectPPP", nan_fn, 0),
        ("sendTextMail", nan_fn, 3),
        ("sendMIMEMail", nan_fn, 3),
        ("getPrefixNumber", null_fn, 0),
        ("setCachePriority", one_fn, 2),
        ("startExtraBrowser", nan_fn, 3),
        ("deleteBookmark", nan_fn, 1),
        ("lockBookmarkArray", nan_fn, 1),
        ("unlockBookmarkArray", nan_fn, 1),
        ("getBookmarkInfo2", null_fn, 1),
        // TR-B14 第三分冊 表 7-20（C プロファイル）の残り。受像機の機能で、規格の「できない」を返す。
        ("setFullDataDisplayArea", one_fn, 1),
        ("saveModule", nan_fn, 2),
        ("saveResource", nan_fn, 2),
        ("saveImageToMemoryCard", nan_fn, 2),
        ("saveHttpServerImageToMemoryCard", nan_fn, 2),
        ("startResidentBookmarkList", nan_fn, 0),
        ("getPrinterStatus", nan_fn, 0),
        ("printFile", nan_fn, 2),
        ("printTemplate", nan_fn, 2),
        ("printUri", nan_fn, 2),
        ("printStaticScreen", nan_fn, 0),
        ("X_DPA_mailTo", nan_fn, 3),
        ("X_DPA_phoneTo", nan_fn, 1),
        ("X_DPA_getRcvCond", nan_fn, 0),
        ("X_DPA_getCurPos", null_fn, 0),
        ("X_DPA_saveExAppFile", nan_fn, 2),
        ("X_DPA_startExAV", nan_fn, 2),
        ("X_DPA_stopExAV", nan_fn, 0),
        ("X_DPA_tuneWithRF", nan_fn, 3),
        ("X_DPA_writeSchInfo", nan_fn, 4),
        ("X_DPA_writeAddressBookInfo", nan_fn, 4),
        ("X_DPA_chkAVtype", nan_fn, 1),
        // STD-B24 第二編 第一分冊 7.2 の残り。規格の「失敗」を返す。引数の数は規格の Syntax の通り。
        ("epgTuneToDocument", nan_fn, 1),
        ("grpIsReserved", nan_fn, 1),
        ("grpReserve", nan_fn, 1),
        ("grpCancelReservation", nan_fn, 1),
        ("grpRecIsReserved", nan_fn, 1),
        ("grpRecReserve", nan_fn, 1),
        ("grpRecCancelReservation", nan_fn, 1),
        ("grpGetNodeEventList", null_fn, 1),
        ("grpGetERTNodeName", null_fn, 1),
        ("grpGetERTNodeDescription", null_fn, 1),
        ("epgXTune", nan_fn, 1),
        ("seriesCancelRecReservation", nan_fn, 2),
        ("setCCStreamReference", nan_fn, 1),
        ("getCCStreamReference", null_fn, 0),
        ("writePersistentString", nan_fn, 3),
        ("writePersistentNumber", nan_fn, 3),
        ("readPersistentString", null_fn, 1),
        ("readPersistentNumber", nan_fn, 1),
        ("copyPersistent", nan_fn, 2),
        ("getPersistentInfoList", null_fn, 1),
        ("deletePersistent", nan_fn, 1),
        ("getFreeSpace", nan_fn, 1),
        ("isSupportedPersistentType", nan_fn, 1),
        ("setAccessInfoOfPersistentArray", nan_fn, 3),
        ("saveDirAs", nan_fn, 3),
        ("saveDir", null_fn, 5),
        ("createDir", nan_fn, 2),
        ("getParentDirName", null_fn, 1),
        ("getDirNames", null_fn, 1),
        ("isDirExisting", false_fn, 1),
        ("saveFileAs", nan_fn, 2),
        ("saveFile", null_fn, 5),
        ("getFileNames", null_fn, 1),
        ("isFileExisting", false_fn, 1),
        ("writeArray", nan_fn, 4),
        ("readArray", null_fn, 2),
        ("getDirInfo", null_fn, 2),
        ("getFileInfo", null_fn, 2),
        ("getContentSource", false_fn, 1),
        ("getStorageInfo", null_fn, 3),
        ("getCarouselInfo", null_fn, 2),
        ("getModuleInfo", null_fn, 2),
        ("saveCarouselAs", nan_fn, 2),
        ("saveCarousel", null_fn, 5),
        ("saveModuleAs", nan_fn, 2),
        ("saveResourceAs", nan_fn, 2),
        ("sendBinaryData", nan_fn, 2),
        ("receiveBinaryData", nan_fn, 2),
        ("registerTransmission", nan_fn, 7),
        ("registerTransmissionStatus", nan_fn, 1),
        ("getTransmissionStatus", nan_fn, 1),
        ("getTransmissionResult", null_fn, 1),
        ("setDelayedTransmissionDataOverBASIC", nan_fn, 8),
        ("setDelayedTransmissionDataOverBasic", nan_fn, 8),
        ("startCASEncryption", nan_fn, 2),
        ("endCASEncryption", nan_fn, 0),
        ("transmitWithCASEncryption", null_fn, 2),
        ("setEncryptionKey", nan_fn, 1),
        ("beginEncryption", nan_fn, 0),
        ("endEncryption", nan_fn, 0),
        ("saveHttpServerFileAs", null_fn, 2),
        ("saveHttpServerFile", null_fn, 5),
        ("sendHttpServerFileAs", null_fn, 2),
        ("saveFtpServerFileAs", nan_fn, 2),
        ("saveFtpServerFile", nan_fn, 5),
        ("sendFtpServerFileAs", null_fn, 2),
        ("setDelayedTransmissionData", nan_fn, 7),
        ("getDelayedTransmissionStatus", null_fn, 0),
        ("getDelayedTransmissionResult", null_fn, 0),
        ("setCacheResourceoverIP", nan_fn, 1),
        ("lockExecution", nan_fn, 0),
        ("unlockExecution", nan_fn, 0),
        ("getLinkSourceServiceStr", null_fn, 0),
        ("getLinkSourceEventStr", null_fn, 0),
        ("getDataDisplayAreaSize", nan_fn, 1),
        ("enumPeripherals", null_fn, 1),
        ("passXMLDocToPeripheral", nan_fn, 3),
        ("getArrayFromPeripheral", null_fn, 4),
        ("lockBookmark", nan_fn, 1),
        ("unlockBookmark", nan_fn, 1),
        ("saveStaticScreenToMemoryCard", nan_fn, 2),
        ("epgStoreReserve", nan_fn, 2),
        ("epgStoreCancelReservation", nan_fn, 1),
        ("epgStoreCheckReservation", nan_fn, 2),
        ("seriesStoreReserve", nan_fn, 2),
        ("seriesStoreCancelReservation", nan_fn, 2),
        ("seriesStoreCheckReservation", nan_fn, 2),
        ("storeStart", nan_fn, 1),
        ("storeTerminate", nan_fn, 1),
        ("checkStoreStatus", nan_fn, 1),
        ("getLicense", null_fn, 5),
        ("getLicenseIDList", null_fn, 4),
        ("getLicenseStatus", null_fn, 4),
        ("setPurchaseInfo", nan_fn, 4),
        ("getPurchaseInfo", null_fn, 1),
        ("getLicenseLinkInfo", null_fn, 1),
        ("getEncryptionkeyInfo", null_fn, 1),
        ("requestCASProcess", null_fn, 2),
        ("getCASInfo", null_fn, 2),
        ("isContentStored", false_fn, 1),
        ("lockStoredContent", nan_fn, 1),
        ("unlockStoredContent", nan_fn, 1),
        ("isLockedStoredContent", false_fn, 1),
        ("deleteStoredContent", nan_fn, 1),
        ("exportContent", null_fn, 2),
        ("launchContent", null_fn, 3),
        ("playSegment", null_fn, 2),
        ("launchDynamicDocument", nan_fn, 2),
        ("getSynopsis", null_fn, 2),
        ("searchMetadata", null_fn, 5),
        ("searchMetadataOnServer", null_fn, 7),
        ("getEntryResourceInformation", null_fn, 1),
        ("downloadContent", nan_fn, 1),
        ("downloadResources", nan_fn, 4),
        ("testNetwork", null_fn, 1),
        ("getHttpResponseHeader", null_fn, 1),
        ("setServerInfo", nan_fn, 4),
        ("getServerInfo", null_fn, 1),
        ("getStorableSpace", nan_fn, 0),
        ("setTune", nan_fn, 1),
        ("getDirStructures", null_fn, 1),
        ("getTransitSource", null_fn, 0),
        // 付属 2 表 11-2 にだけあるもの（スマートデバイス、IPTV、綴りの違い）。
        ("transmitDataToSmartDevice", nan_fn, 2),
        ("startDlcDownload", nan_fn, 1),
        ("getDlcDownloadStatus", null_fn, 1),
        ("startVOD", nan_fn, 1),
        ("epgCancelRecReservation", nan_fn, 1),
    ] {
        object.function(NativeFunction::from_fn_ptr(f), JsString::from(name), length);
    }
    object.build()
}

fn register_array(context: &mut Context, user: bool) -> JsValue {
    let mut object = ObjectInitializer::new(context);
    object.property(js_string!("__u"), JsValue::from(user), Attribute::all());
    for index in 0..64u32 {
        let get = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(
                |this, _, index, context| register_get(this, *index, context),
                index,
            ),
        )
        .build();
        let set = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(
                |this, args, index, context| register_set(this, args, *index, context),
                index,
            ),
        )
        .build();
        object.accessor(index, Some(get), Some(set), Attribute::all());
    }
    object.property(js_string!("length"), JsValue::from(64), Attribute::all());
    object.build().into()
}

fn is_user(this: &JsValue, context: &mut Context) -> bool {
    this.as_object()
        .and_then(|object| object.get(js_string!("__u"), context).ok())
        .is_some_and(|value| value.to_boolean())
}

fn register_get(this: &JsValue, index: u32, context: &mut Context) -> JsResult<JsValue> {
    let user = is_user(this, context);
    let host = host(context);
    let registers = if user { &host.ureg } else { &host.greg };
    Ok(js(registers.get(index as usize).map_or("", String::as_str)))
}

fn register_set(
    this: &JsValue,
    args: &[JsValue],
    index: u32,
    context: &mut Context,
) -> JsResult<JsValue> {
    let user = is_user(this, context);
    let value = string(args.first(), context);
    let host = host(context);
    let registers = if user { &mut host.ureg } else { &mut host.greg };
    if let Some(slot) = registers.get_mut(index as usize) {
        *slot = value;
    }
    Ok(JsValue::undefined())
}

fn nan_fn(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    nan()
}

/// `X_DPA_getComBrowserUA()`（C プロファイル）。NaN だと `c[0][0]` でエラーになるので `00` と空の UA を返す。
fn com_browser_ua(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let inner = JsArray::from_iter(
        [
            JsValue::from(js_string!("00")),
            JsValue::from(js_string!("")),
        ],
        context,
    );
    Ok(JsArray::from_iter([JsValue::from(inner)], context).into())
}

fn null_fn(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::null())
}

pub(super) fn false_fn(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(false))
}

fn one_fn(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(1))
}

fn format_number(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = number(args.first(), context);
    if !value.is_finite() {
        return Ok(JsValue::null());
    }
    let whole = value.trunc() as i64;
    let digits = whole.abs().to_string();
    let mut out = String::new();
    for (index, letter) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(letter);
    }
    if whole < 0 {
        out.insert(0, '-');
    }
    Ok(js(&out))
}
