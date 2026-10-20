#include "napi/native_api.h"
#include "bride.h"
#include "libclash.h"
#include <cstring>
#include <cstdlib>
#include <future>

enum class InterfaceType {
    Invoke,
    Tun,
};

struct TunInterface {
    InterfaceType type = InterfaceType::Tun;
    napi_threadsafe_function protect;
    napi_threadsafe_function resolveUid;
    napi_threadsafe_function resolvePackage;
};

struct InvokeInterface {
    InterfaceType type = InterfaceType::Invoke;
    napi_threadsafe_function onResult;
};

struct ResolveUidCall {
    int protocol;
    const char *source;
    const char *target;
    std::promise<int> promise;
};

struct ResolvePackageCall {
    int uid;
    std::promise<char*> promise;
};

struct ProtectCall {
    int fd;
    std::promise<bool> promise;
};

static char *GetValueStringUtf8(napi_env env, napi_value value) {
    size_t length = 0;
    napi_get_value_string_utf8(env, value, nullptr, 0, &length);
    char *result = static_cast<char*>(malloc(length + 1));
    napi_get_value_string_utf8(env, value, result, length + 1, &length);
    return result;
}

static int GetValueInt32(napi_env env, napi_value value) {
    int result = 0;
    napi_get_value_int32(env, value, &result);
    return result;
}

static bool GetValueBool(napi_env env, napi_value value) {
    bool result = false;
    napi_get_value_bool(env, value, &result);
    return result;
}

static napi_value CreateValueStringUtf8(napi_env env, const char *data) {
    napi_value result = nullptr;
    napi_create_string_utf8(env, data, NAPI_AUTO_LENGTH, &result);
    return result;
}

static napi_value CreateValueInt32(napi_env env, int data) {
    napi_value result = nullptr;
    napi_create_int32(env, data, &result);
    return result;
}


void InvokeMethodCallJsCallback(napi_env env, napi_value callback, void* context, void* data){
    napi_value global = nullptr;
    napi_get_global(env, &global);
    napi_value argv = CreateValueStringUtf8(env, reinterpret_cast<char*>(data));
    napi_value result = nullptr;
    napi_call_function(env, global, callback, 1, &argv, &result);
    free(data);
}

static napi_value ProtectSucceeded(napi_env env, napi_callback_info info) {
    void *data = nullptr;
    napi_get_cb_info(env, info, nullptr, nullptr, nullptr, &data);
    reinterpret_cast<ProtectCall*>(data)->promise.set_value(true);
    return nullptr;
}

static napi_value ProtectFailed(napi_env env, napi_callback_info info) {
    void *data = nullptr;
    napi_get_cb_info(env, info, nullptr, nullptr, nullptr, &data);
    reinterpret_cast<ProtectCall*>(data)->promise.set_value(false);
    return nullptr;
}

static void ClearPendingException(napi_env env) {
    bool pending = false;
    napi_is_exception_pending(env, &pending);
    if (pending) {
        napi_value exception = nullptr;
        napi_get_and_clear_last_exception(env, &exception);
    }
}

void ProtectCallJsCallback(napi_env env, napi_value callback, void *context, void *data) {
    auto *call = reinterpret_cast<ProtectCall*>(data);
    if (!env || !callback) { call->promise.set_value(false); return; }
    napi_value global = nullptr, result = nullptr, then = nullptr;
    napi_get_global(env, &global);
    napi_value arg = CreateValueInt32(env, call->fd);
    if (napi_call_function(env, global, callback, 1, &arg, &result) != napi_ok ||
        napi_get_named_property(env, result, "then", &then) != napi_ok) {
        ClearPendingException(env);
        call->promise.set_value(false);
        return;
    }
    napi_value callbacks[2] = {nullptr, nullptr};
    napi_create_function(env, "protectSucceeded", NAPI_AUTO_LENGTH, ProtectSucceeded, call, &callbacks[0]);
    napi_create_function(env, "protectFailed", NAPI_AUTO_LENGTH, ProtectFailed, call, &callbacks[1]);
    napi_value ignored = nullptr;
    if (napi_call_function(env, result, then, 2, callbacks, &ignored) != napi_ok) {
        ClearPendingException(env);
        call->promise.set_value(false);
    }
}

void ResolveUidCallJsCallback(napi_env env, napi_value callback, void *context, void *data) {
    auto *call = reinterpret_cast<ResolveUidCall*>(data);
    if (!env || !callback) { call->promise.set_value(-1); return; }
    napi_value global = nullptr, result = nullptr;
    napi_get_global(env, &global);
    napi_value args[3] = {CreateValueInt32(env, call->protocol), CreateValueStringUtf8(env, call->source), CreateValueStringUtf8(env, call->target)};
    int uid = -1;
    if (napi_call_function(env, global, callback, 3, args, &result) == napi_ok) {
        napi_get_value_int32(env, result, &uid);
    }
    ClearPendingException(env);
    call->promise.set_value(uid);
}

void ResolvePackageCallJsCallback(napi_env env, napi_value callback, void *context, void *data) {
    auto *call = reinterpret_cast<ResolvePackageCall*>(data);
    if (!env || !callback) { call->promise.set_value(nullptr); return; }
    napi_value global = nullptr, result = nullptr;
    napi_get_global(env, &global);
    napi_value arg = CreateValueInt32(env, call->uid);
    char *name = nullptr;
    if (napi_call_function(env, global, callback, 1, &arg, &result) == napi_ok) {
        name = GetValueStringUtf8(env, result);
    }
    ClearPendingException(env);
    call->promise.set_value(name);
}

static napi_value InvokeMethod(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    InvokeInterface *invokeInterface = new InvokeInterface();
    napi_create_threadsafe_function(env, argv[0], nullptr, CreateValueStringUtf8(env, "onResult"), 0, 1, nullptr, nullptr, nullptr, InvokeMethodCallJsCallback, &invokeInterface->onResult);
    invokeMethod(invokeInterface, GetValueStringUtf8(env, argv[1]));
    return nullptr;
}

static napi_value StartTUN(napi_env env, napi_callback_info info) {
    size_t argc = 7;
    napi_value argv[7] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    auto *tunInterface = new TunInterface();
    napi_create_threadsafe_function(env, argv[0], nullptr, CreateValueStringUtf8(env, "protect"), 0, 1, nullptr, nullptr, nullptr, ProtectCallJsCallback, &tunInterface->protect);
    napi_create_threadsafe_function(env, argv[1], nullptr, CreateValueStringUtf8(env, "resolveUid"), 0, 1, nullptr, nullptr, nullptr, ResolveUidCallJsCallback, &tunInterface->resolveUid);
    napi_create_threadsafe_function(env, argv[2], nullptr, CreateValueStringUtf8(env, "resolvePackage"), 0, 1, nullptr, nullptr, nullptr, ResolvePackageCallJsCallback, &tunInterface->resolvePackage);
    bool started = startTUN(tunInterface, GetValueInt32(env, argv[3]), GetValueStringUtf8(env, argv[4]), GetValueStringUtf8(env, argv[5]), GetValueStringUtf8(env, argv[6]));
    napi_value result = nullptr;
    napi_get_boolean(env, started, &result);
    return result;
}

static napi_value QuickSetup(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    InvokeInterface *invokeInterface = new InvokeInterface();
    napi_create_threadsafe_function(env, argv[0], nullptr, CreateValueStringUtf8(env, "onResult"), 0, 1, nullptr, nullptr, nullptr, InvokeMethodCallJsCallback, &invokeInterface->onResult);
    quickSetup(invokeInterface, GetValueStringUtf8(env, argv[1]), GetValueStringUtf8(env, argv[2]));
    return nullptr;
}

static napi_value SetEventListener(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    napi_valuetype type = napi_undefined;
    if (argc > 0) {
        napi_typeof(env, argv[0], &type);
    }
    if (argc == 0 || type == napi_undefined || type == napi_null) {
        setEventListener(nullptr);
        return nullptr;
    }
    InvokeInterface *invokeInterface = new InvokeInterface();
    napi_create_threadsafe_function(env, argv[0], nullptr, CreateValueStringUtf8(env, "onResult"), 0, 1, nullptr, nullptr, nullptr, InvokeMethodCallJsCallback, &invokeInterface->onResult);
    setEventListener(invokeInterface);
    return nullptr;
}

static napi_value GetTotalTraffic(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    char *data = getTotalTraffic(GetValueBool(env, argv[0]));
    return CreateValueStringUtf8(env, data);
}

static napi_value GetTraffic(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    char *data = getTraffic(GetValueBool(env, argv[0]));
    return CreateValueStringUtf8(env, data);
}

static napi_value StopTun(napi_env env, napi_callback_info info) {
    stopTun();
    return nullptr;
}

static napi_value ForceGC(napi_env env, napi_callback_info info) {
    forceGC();
    return nullptr;
}

static napi_value UpdateDns(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    updateDns(GetValueStringUtf8(env, argv[0]));
    return nullptr;
}

static int call_tun_interface_protect_impl(void *tun_interface, const int fd) {
    auto *tunInterface = reinterpret_cast<TunInterface*>(tun_interface);
    ProtectCall call{fd, {}};
    auto future = call.promise.get_future();
    if (napi_call_threadsafe_function(tunInterface->protect, &call, napi_tsfn_nonblocking) != napi_ok) return 0;
    return future.get() ? 1 : 0;
}

static int call_tun_interface_resolve_uid_impl(void *tun_interface, int protocol, const char *source, const char *target) {
    auto *tunInterface = reinterpret_cast<TunInterface*>(tun_interface);
    ResolveUidCall call{protocol, source, target, {}};
    auto future = call.promise.get_future();
    if (napi_call_threadsafe_function(tunInterface->resolveUid, &call, napi_tsfn_nonblocking) != napi_ok) return -1;
    return future.get();
}

static char *call_tun_interface_resolve_package_impl(void *tun_interface, int uid) {
    auto *tunInterface = reinterpret_cast<TunInterface*>(tun_interface);
    ResolvePackageCall call{uid, {}};
    auto future = call.promise.get_future();
    if (napi_call_threadsafe_function(tunInterface->resolvePackage, &call, napi_tsfn_nonblocking) != napi_ok) return nullptr;
    return future.get();
}

static void call_invoke_interface_result_impl(void *invoke_interface, const char *data) {
    InvokeInterface *invokeInterface = reinterpret_cast<InvokeInterface*>(invoke_interface);
    char *dataCopy = nullptr;
    if (data != nullptr) {
        size_t length = std::strlen(data);
        dataCopy = static_cast<char*>(malloc(length + 1));
        std::memcpy(dataCopy, data, length + 1);
    }
    napi_call_threadsafe_function(invokeInterface->onResult, dataCopy, napi_tsfn_nonblocking);
}

static void release_object_impl(void *obj) {
    if (obj == nullptr) {
        return;
    }
    if (*reinterpret_cast<InterfaceType*>(obj) == InterfaceType::Tun) {
        TunInterface *tunInterface = reinterpret_cast<TunInterface*>(obj);
        napi_release_threadsafe_function(tunInterface->protect, napi_tsfn_release);
        napi_release_threadsafe_function(tunInterface->resolveUid, napi_tsfn_release);
        napi_release_threadsafe_function(tunInterface->resolvePackage, napi_tsfn_release);
        delete tunInterface;
        return;
    }
    InvokeInterface *invokeInterface = reinterpret_cast<InvokeInterface*>(obj);
    napi_release_threadsafe_function(invokeInterface->onResult, napi_tsfn_release);
    delete invokeInterface;
}

static void free_string_impl(char *data) {
    free(data);
}


EXTERN_C_START
static napi_value Init(napi_env env, napi_value exports)
{
    napi_property_descriptor desc[] = {
        { "invokeMethod", nullptr, InvokeMethod, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "startTUN", nullptr, StartTUN, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "quickSetup", nullptr, QuickSetup, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "setEventListener", nullptr, SetEventListener, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "getTotalTraffic", nullptr, GetTotalTraffic, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "getTraffic", nullptr, GetTraffic, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "stopTun", nullptr, StopTun, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "forceGC", nullptr, ForceGC, nullptr, nullptr, nullptr, napi_default, nullptr },
        { "updateDns", nullptr, UpdateDns, nullptr, nullptr, nullptr, napi_default, nullptr }
    };
    napi_define_properties(env, exports, sizeof(desc) / sizeof(desc[0]), desc);

    protect_func = &call_tun_interface_protect_impl;
    resolve_uid_func = &call_tun_interface_resolve_uid_impl;
    resolve_package_func = &call_tun_interface_resolve_package_impl;
    result_func = &call_invoke_interface_result_impl;
    release_object_func = &release_object_impl;
    free_string_func = &free_string_impl;

    return exports;
}
EXTERN_C_END

static napi_module coreModule = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = nullptr,
    .nm_register_func = Init,
    .nm_modname = "core",
    .nm_priv = ((void*)0),
    .reserved = {0},
};

extern "C" __attribute__((constructor)) void RegisterCoreModule(void)
{
    napi_module_register(&coreModule);
}
