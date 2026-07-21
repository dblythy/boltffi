#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdatomic.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    int32_t code;
} FfiStatus;

#define FFI_STATUS_OK ((FfiStatus){0})
#define FFI_STATUS_NULL_POINTER ((FfiStatus){1})
#define FFI_STATUS_BUFFER_TOO_SMALL ((FfiStatus){2})
#define FFI_STATUS_INVALID_ARG ((FfiStatus){3})
#define FFI_STATUS_CANCELLED ((FfiStatus){4})
#define FFI_STATUS_INTERNAL_ERROR ((FfiStatus){100})

typedef struct {
    uint8_t *ptr;
    uintptr_t len;
    uintptr_t cap;
    uintptr_t align;
} FfiBuf_u8;

typedef struct {
    uint8_t *ptr;
    uintptr_t len;
    uintptr_t cap;
} FfiString;

typedef struct {
    FfiString message;
} FfiError;

typedef struct {
    const uint8_t *ptr;
    uintptr_t len;
} FfiSpan;

typedef const void *RustFutureHandle;
typedef int8_t StreamPollResult;
typedef int32_t WaitResult;
typedef void (*RustFutureContinuationCallback)(uint64_t callback_data, int8_t poll_result);
typedef void (*StreamContinuationCallback)(uint64_t callback_data, StreamPollResult result);

static inline bool boltffi_atomic_u8_cas(uint8_t *state, uint8_t expected, uint8_t desired) {
    return atomic_compare_exchange_strong_explicit((_Atomic uint8_t *)state, &expected, desired, memory_order_acq_rel, memory_order_acquire);
}

static inline uint64_t boltffi_atomic_u64_exchange(uint64_t *slot, uint64_t value) {
    return atomic_exchange_explicit((_Atomic uint64_t *)slot, value, memory_order_acq_rel);
}

static inline bool boltffi_atomic_u64_cas(uint64_t *slot, uint64_t expected, uint64_t desired) {
    return atomic_compare_exchange_strong_explicit((_Atomic uint64_t *)slot, &expected, desired, memory_order_acq_rel, memory_order_acquire);
}

static inline uint64_t boltffi_atomic_u64_load(uint64_t *slot) {
    return atomic_load_explicit((_Atomic uint64_t *)slot, memory_order_acquire);
}

typedef struct {
    uint64_t handle;
    const void *vtable;
} BoltFFICallbackHandle;

void boltffi_free_string(FfiString string);
void boltffi_free_buf(FfiBuf_u8 buf);
FfiBuf_u8 boltffi_buf_from_bytes(const uint8_t *ptr, uintptr_t len);
FfiBuf_u8 boltffi_buf_with_len(uintptr_t len);
FfiStatus boltffi_last_error_message(FfiString *out);
void boltffi_clear_last_error(void);
typedef int32_t ___NetworkState;
#define NETWORK_STATE_UNKNOWN ((___NetworkState)0)
#define NETWORK_STATE_ONLINE ((___NetworkState)1)
#define NETWORK_STATE_OFFLINE ((___NetworkState)2)
typedef uint32_t ___ParseValue;
#define PARSE_VALUE_NULL ((___ParseValue)0)
#define PARSE_VALUE_BOOL ((___ParseValue)1)
#define PARSE_VALUE_INT ((___ParseValue)2)
#define PARSE_VALUE_DOUBLE ((___ParseValue)3)
#define PARSE_VALUE_STR ((___ParseValue)4)
#define PARSE_VALUE_DATE ((___ParseValue)5)
#define PARSE_VALUE_BYTES ((___ParseValue)6)
#define PARSE_VALUE_GEO ((___ParseValue)7)
#define PARSE_VALUE_FILE ((___ParseValue)8)
#define PARSE_VALUE_POINTER ((___ParseValue)9)
#define PARSE_VALUE_OBJECT ((___ParseValue)10)
#define PARSE_VALUE_POLYGON ((___ParseValue)11)
#define PARSE_VALUE_RELATION ((___ParseValue)12)
#define PARSE_VALUE_ARRAY ((___ParseValue)13)
#define PARSE_VALUE_MAP ((___ParseValue)14)
typedef uint32_t ___ProviderAuthData;
#define PROVIDER_AUTH_DATA_APPLE ((___ProviderAuthData)0)
#define PROVIDER_AUTH_DATA_GOOGLE ((___ProviderAuthData)1)
#define PROVIDER_AUTH_DATA_FACEBOOK ((___ProviderAuthData)2)
#define PROVIDER_AUTH_DATA_CUSTOM ((___ProviderAuthData)3)
typedef int32_t ___CachePolicy;
#define CACHE_POLICY_NETWORK_ONLY ((___CachePolicy)0)
#define CACHE_POLICY_CACHE_ELSE_NETWORK ((___CachePolicy)1)
#define CACHE_POLICY_CACHE_THEN_NETWORK ((___CachePolicy)2)
typedef uint32_t ___CollectionDiff;
#define COLLECTION_DIFF_INSERT ((___CollectionDiff)0)
#define COLLECTION_DIFF_UPDATE ((___CollectionDiff)1)
#define COLLECTION_DIFF_MOVE ((___CollectionDiff)2)
#define COLLECTION_DIFF_REMOVE ((___CollectionDiff)3)
#define COLLECTION_DIFF_RESET ((___CollectionDiff)4)
#define COLLECTION_DIFF_NEEDS_RESYNC ((___CollectionDiff)5)
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*send)(uint64_t, const uint8_t *, uintptr_t);
    void (*schedule)(uint64_t, int64_t, uint64_t);
    void (*open_socket)(uint64_t);
    void (*close_socket)(uint64_t);
} ___LiveQueryTransportVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*on_event)(uint64_t, const uint8_t *, uintptr_t);
} ___LiveQueryListenerVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*fetch)(uint64_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus, FfiBuf_u8), void *);
} ___HttpTransportVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    FfiBuf_u8 (*fill)(uint64_t, uint32_t);
} ___RandomSourceVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*schedule)(uint64_t, int64_t, uint64_t);
} ___TimerVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    uint64_t (*now_ms)(uint64_t);
} ___ClockVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*get)(uint64_t, void (*)(void *, FfiStatus, FfiBuf_u8), void *);
    void (*set)(uint64_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus), void *);
    void (*clear)(uint64_t, void (*)(void *, FfiStatus), void *);
} ___SessionStorageVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*get)(uint64_t, void (*)(void *, FfiStatus, FfiBuf_u8), void *);
    void (*set)(uint64_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus), void *);
    void (*clear)(uint64_t, void (*)(void *, FfiStatus), void *);
} ___InstallationIdStorageVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*get)(uint64_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus, FfiBuf_u8), void *);
    void (*set)(uint64_t, const uint8_t *, uintptr_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus), void *);
    void (*delete)(uint64_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus), void *);
    void (*keys)(uint64_t, const uint8_t *, uintptr_t, void (*)(void *, FfiStatus, FfiBuf_u8), void *);
} ___KeyValueStorageVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*on_dropped)(uint64_t, const uint8_t *, uintptr_t, const uint8_t *, uintptr_t, int32_t, const uint8_t *, uintptr_t);
} ___EventuallyQueueListenerVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*on_event)(uint64_t, const uint8_t *, uintptr_t);
} ___ObservabilitySinkVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*on_diff)(uint64_t, const uint8_t *, uintptr_t);
} ___WatchListenerVTable;
typedef struct {
    void (*free)(uint64_t);
    uint64_t (*clone)(uint64_t);
    void (*on_change)(uint64_t, ___NetworkState);
} ___NetworkStateListenerVTable;
void boltffi_register_callback_parse_core_ffi_livequery_live_query_transport(const ___LiveQueryTransportVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_livequery_live_query_transport(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_livequery_live_query_listener(const ___LiveQueryListenerVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_livequery_live_query_listener(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_http_transport(const ___HttpTransportVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_http_transport(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_random_source(const ___RandomSourceVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_random_source(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_timer(const ___TimerVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_timer(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_clock(const ___ClockVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_clock(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_session_storage(const ___SessionStorageVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_session_storage(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_installation_id_storage(const ___InstallationIdStorageVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_installation_id_storage(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_key_value_storage(const ___KeyValueStorageVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_key_value_storage(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_eventually_queue_listener(const ___EventuallyQueueListenerVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_eventually_queue_listener(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_transport_observability_sink(const ___ObservabilitySinkVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_transport_observability_sink(uint64_t handle);
void boltffi_register_callback_parse_core_ffi_watch_watch_listener(const ___WatchListenerVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_ffi_watch_watch_listener(uint64_t handle);
void boltffi_register_callback_parse_core_network_network_state_listener(const ___NetworkStateListenerVTable *vtable);
BoltFFICallbackHandle boltffi_create_callback_parse_core_network_network_state_listener(uint64_t handle);
FfiBuf_u8 boltffi_init_record_parse_core_ffi_acl_acl_new(void);
FfiBuf_u8 boltffi_init_record_parse_core_ffi_acl_acl_owner(const uint8_t *user_id_ptr, uintptr_t user_id_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_acl_acl_set_read_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *id_ptr, uintptr_t id_len, bool allowed);
bool boltffi_method_record_parse_core_ffi_acl_acl_get_read_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *id_ptr, uintptr_t id_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_acl_acl_set_write_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *id_ptr, uintptr_t id_len, bool allowed);
bool boltffi_method_record_parse_core_ffi_acl_acl_get_write_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *id_ptr, uintptr_t id_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_acl_acl_set_public_read_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, bool allowed);
bool boltffi_method_record_parse_core_ffi_acl_acl_get_public_read_access(const uint8_t *receiver_ptr, uintptr_t receiver_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_acl_acl_set_public_write_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, bool allowed);
bool boltffi_method_record_parse_core_ffi_acl_acl_get_public_write_access(const uint8_t *receiver_ptr, uintptr_t receiver_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_acl_acl_set_role_read_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *role_ptr, uintptr_t role_len, bool allowed);
bool boltffi_method_record_parse_core_ffi_acl_acl_get_role_read_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *role_ptr, uintptr_t role_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_acl_acl_set_role_write_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *role_ptr, uintptr_t role_len, bool allowed);
bool boltffi_method_record_parse_core_ffi_acl_acl_get_role_write_access(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *role_ptr, uintptr_t role_len);
FfiBuf_u8 boltffi_init_record_parse_core_ffi_client_request_options_new(void);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_master_key(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *master_key_ptr, uintptr_t master_key_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_session_token(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *session_token_ptr, uintptr_t session_token_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_context_json(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *context_json_ptr, uintptr_t context_json_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_read_preference(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *read_preference_ptr, uintptr_t read_preference_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_transaction(const uint8_t *receiver_ptr, uintptr_t receiver_len, bool transaction);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_timeout_ms(const uint8_t *receiver_ptr, uintptr_t receiver_len, int64_t timeout_ms);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_cancel_id(const uint8_t *receiver_ptr, uintptr_t receiver_len, const uint8_t *cancel_id_ptr, uintptr_t cancel_id_len);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_maintenance_key(const uint8_t *receiver_ptr, uintptr_t receiver_len, bool use_maintenance_key);
FfiBuf_u8 boltffi_method_record_parse_core_ffi_client_request_options_with_cascade_save(const uint8_t *receiver_ptr, uintptr_t receiver_len, bool cascade_save);
void boltffi_release_class_parse_core_ffi_batch_batch_builder(uint64_t handle);
FfiBuf_u8 boltffi_init_class_parse_core_ffi_batch_batch_builder_new(const uint8_t *client_id_ptr, uintptr_t client_id_len, uint64_t *return_out);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_batch_batch_builder_create(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *data_ptr, uintptr_t data_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_batch_batch_builder_update(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *object_id_ptr, uintptr_t object_id_len, const uint8_t *data_ptr, uintptr_t data_len);
FfiStatus boltffi_method_class_parse_core_ffi_batch_batch_builder_delete(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *object_id_ptr, uintptr_t object_id_len);
FfiStatus boltffi_method_class_parse_core_ffi_batch_batch_builder_get(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *object_id_ptr, uintptr_t object_id_len);
int64_t boltffi_method_class_parse_core_ffi_batch_batch_builder_len(uint64_t receiver);
bool boltffi_method_class_parse_core_ffi_batch_batch_builder_is_empty(uint64_t receiver);
RustFutureHandle boltffi_method_class_parse_core_ffi_batch_batch_builder_execute(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_batch_batch_builder_execute_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_batch_batch_builder_execute_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_batch_batch_builder_execute_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_batch_batch_builder_execute_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_batch_batch_builder_execute_free(RustFutureHandle handle);
void boltffi_release_class_parse_core_ffi_cancel_cancellation_source(uint64_t handle);
uint64_t boltffi_init_class_parse_core_ffi_cancel_cancellation_source_new(void);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_cancel_cancellation_source_id(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_cancel_cancellation_source_cancel(uint64_t receiver);
bool boltffi_method_class_parse_core_ffi_cancel_cancellation_source_is_cancelled(uint64_t receiver);
void boltffi_release_class_parse_core_ffi_client_parse_client(uint64_t handle);
uint64_t boltffi_init_class_parse_core_ffi_client_parse_client_new(const uint8_t *config_ptr, uintptr_t config_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_id(uint64_t receiver);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_sdk(uint64_t receiver, bool cleanup);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_sdk_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_sdk_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_sdk_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_sdk_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_sdk_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_lite(uint64_t receiver, bool cleanup);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_lite_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_lite_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_lite_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_lite_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_migrate_from_parse_lite_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_set_allow_custom_object_id(uint64_t receiver, bool allow);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_set_maintenance_key(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_client_parse_client_release(uint64_t receiver);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_save_all(uint64_t receiver, const uint8_t *local_ids_ptr, uintptr_t local_ids_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_all_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_save_all_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_save_all_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_all_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_all_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_destroy_all(uint64_t receiver, const uint8_t *local_ids_ptr, uintptr_t local_ids_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_all_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_all_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_all_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_all_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_all_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_pin(uint64_t receiver, const uint8_t *local_ids_ptr, uintptr_t local_ids_len, const uint8_t *name_ptr, uintptr_t name_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_pin_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_pin_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_pin_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_pin_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_pin_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_unpin(uint64_t receiver, const uint8_t *local_ids_ptr, uintptr_t local_ids_len, const uint8_t *name_ptr, uintptr_t name_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_invalidate_query_cache(uint64_t receiver, const uint8_t *class_ptr, uintptr_t class_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_invalidate_query_cache_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_invalidate_query_cache_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_invalidate_query_cache_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_invalidate_query_cache_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_invalidate_query_cache_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_unpin_all_objects(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_all_objects_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_all_objects_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_all_objects_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_all_objects_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unpin_all_objects_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_save_eventually(uint64_t receiver, const uint8_t *local_ids_ptr, uintptr_t local_ids_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_eventually_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_save_eventually_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_save_eventually_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_eventually_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_eventually_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_destroy_eventually(uint64_t receiver, const uint8_t *local_ids_ptr, uintptr_t local_ids_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_eventually_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_eventually_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_eventually_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_eventually_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_destroy_eventually_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_flush_eventually(uint64_t receiver);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_flush_eventually_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_flush_eventually_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_flush_eventually_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_flush_eventually_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_flush_eventually_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_run(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *params_ptr, uintptr_t params_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_run_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_run_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_run_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_run_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_run_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_run_job(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *params_ptr, uintptr_t params_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_run_job_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_run_job_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_run_job_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_run_job_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_run_job_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_send_push(uint64_t receiver, const uint8_t *push_ptr, uintptr_t push_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_send_push_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_send_push_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_send_push_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_send_push_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_send_push_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_set_pinned_certificates(uint64_t receiver, const uint8_t *pems_ptr, uintptr_t pems_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_network_state(uint64_t receiver, ___NetworkState *return_out);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_set_network_reachable(uint64_t receiver, bool reachable, bool *return_out);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_probe_network(uint64_t receiver);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_probe_network_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_probe_network_complete(RustFutureHandle handle, FfiStatus *out_status, ___NetworkState *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_probe_network_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_probe_network_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_probe_network_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_create_audience(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *where_json_ptr, uintptr_t where_json_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_audience_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_audience_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_audience_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_audience_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_audience_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_audience(uint64_t receiver, const uint8_t *id_ptr, uintptr_t id_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_audience_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_audience_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_audience_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_audience_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_audience_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_update_audience(uint64_t receiver, const uint8_t *id_ptr, uintptr_t id_len, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *where_json_ptr, uintptr_t where_json_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_audience_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_audience_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_audience_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_audience_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_audience_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_delete_audience(uint64_t receiver, const uint8_t *id_ptr, uintptr_t id_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_audience_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_audience_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_audience_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_audience_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_audience_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_list_audiences(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_list_audiences_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_list_audiences_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_list_audiences_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_list_audiences_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_list_audiences_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_function_hooks(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hooks_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hooks_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hooks_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hooks_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hooks_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_function_hook(uint64_t receiver, const uint8_t *function_name_ptr, uintptr_t function_name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hook_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_function_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_create_function_hook(uint64_t receiver, const uint8_t *function_name_ptr, uintptr_t function_name_len, const uint8_t *url_ptr, uintptr_t url_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_function_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_function_hook_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_function_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_function_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_function_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_update_function_hook(uint64_t receiver, const uint8_t *function_name_ptr, uintptr_t function_name_len, const uint8_t *url_ptr, uintptr_t url_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_function_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_function_hook_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_function_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_function_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_function_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_delete_function_hook(uint64_t receiver, const uint8_t *function_name_ptr, uintptr_t function_name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_function_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_function_hook_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_function_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_function_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_function_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_trigger_hooks(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hooks_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hooks_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hooks_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hooks_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hooks_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_trigger_hook(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *trigger_name_ptr, uintptr_t trigger_name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hook_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_trigger_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_create_trigger_hook(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *trigger_name_ptr, uintptr_t trigger_name_len, const uint8_t *url_ptr, uintptr_t url_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_trigger_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_trigger_hook_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_trigger_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_trigger_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_trigger_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_update_trigger_hook(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *trigger_name_ptr, uintptr_t trigger_name_len, const uint8_t *url_ptr, uintptr_t url_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_trigger_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_trigger_hook_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_trigger_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_trigger_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_trigger_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_delete_trigger_hook(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *trigger_name_ptr, uintptr_t trigger_name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_trigger_hook_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_trigger_hook_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_trigger_hook_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_trigger_hook_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_trigger_hook_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_sign_up(uint64_t receiver, const uint8_t *username_ptr, uintptr_t username_len, const uint8_t *password_ptr, uintptr_t password_len, const uint8_t *extra_ptr, uintptr_t extra_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_sign_up_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_sign_up_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_sign_up_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_sign_up_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_sign_up_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_log_in(uint64_t receiver, const uint8_t *username_ptr, uintptr_t username_len, const uint8_t *password_ptr, uintptr_t password_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_log_out(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_out_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_out_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_out_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_out_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_out_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_become_user(uint64_t receiver, const uint8_t *session_token_ptr, uintptr_t session_token_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_become_user_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_become_user_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_become_user_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_become_user_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_become_user_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_verify_password(uint64_t receiver, const uint8_t *username_ptr, uintptr_t username_len, const uint8_t *password_ptr, uintptr_t password_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_verify_password_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_verify_password_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_verify_password_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_verify_password_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_verify_password_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_log_in_with(uint64_t receiver, const uint8_t *data_ptr, uintptr_t data_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_with_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_with_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_with_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_with_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_with_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_log_in_anonymously(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_anonymously_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_anonymously_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_anonymously_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_anonymously_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_anonymously_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_link(uint64_t receiver, const uint8_t *data_ptr, uintptr_t data_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_link_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_link_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_link_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_link_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_link_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_unlink(uint64_t receiver, const uint8_t *provider_ptr, uintptr_t provider_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unlink_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_unlink_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_unlink_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unlink_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_unlink_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_current_user(uint64_t receiver, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_current_user_row(uint64_t receiver, FfiBuf_u8 *return_out);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_current_session(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_current_session_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_current_session_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_current_session_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_current_session_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_current_session_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_request_password_reset(uint64_t receiver, const uint8_t *email_ptr, uintptr_t email_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_request_password_reset_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_request_password_reset_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_request_password_reset_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_request_password_reset_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_request_password_reset_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_request_email_verification(uint64_t receiver, const uint8_t *email_ptr, uintptr_t email_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_request_email_verification_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_request_email_verification_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_request_email_verification_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_request_email_verification_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_request_email_verification_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_upload_file(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *data_ptr, uintptr_t data_len, const uint8_t *content_type_ptr, uintptr_t content_type_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_upload_file_text(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *text_ptr, uintptr_t text_len, const uint8_t *content_type_ptr, uintptr_t content_type_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_text_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_text_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_text_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_text_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_upload_file_text_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_config(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_config_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_config_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_config_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_config_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_config_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_save_config(uint64_t receiver, const uint8_t *params_ptr, uintptr_t params_len, const uint8_t *master_key_only_ptr, uintptr_t master_key_only_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_config_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_save_config_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_save_config_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_config_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_save_config_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_track_event(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *dimensions_ptr, uintptr_t dimensions_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_track_event_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_track_event_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_track_event_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_track_event_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_track_event_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_set_request_id_prefix(uint64_t receiver, const uint8_t *prefix_ptr, uintptr_t prefix_len);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_restore_session(uint64_t receiver);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_restore_session_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_restore_session_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_restore_session_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_restore_session_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_restore_session_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_ensure_installation_id(uint64_t receiver);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_ensure_installation_id_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_ensure_installation_id_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_ensure_installation_id_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_ensure_installation_id_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_ensure_installation_id_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_installation_id(uint64_t receiver, FfiBuf_u8 *return_out);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_server_logs(uint64_t receiver, const uint8_t *query_ptr, uintptr_t query_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_logs_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_server_logs_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_server_logs_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_logs_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_logs_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_make_file(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *url_ptr, uintptr_t url_len, FfiBuf_u8 *return_out);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_delete_file(uint64_t receiver, const uint8_t *name_ptr, uintptr_t name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_file_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_file_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_file_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_file_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_file_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_log_in_as(uint64_t receiver, const uint8_t *user_id_ptr, uintptr_t user_id_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_as_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_as_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_as_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_as_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_log_in_as_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_server_health(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_health_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_server_health_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_server_health_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_health_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_health_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_server_info(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_info_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_server_info_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_server_info_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_info_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_server_info_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_schemas(uint64_t receiver);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schemas_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schemas_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schemas_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schemas_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schemas_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_get_schema(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schema_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schema_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schema_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schema_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_get_schema_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_register_schema(uint64_t receiver, const uint8_t *schema_ptr, uintptr_t schema_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_client_parse_client_set_schema_validation(uint64_t receiver, bool enabled);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_create_schema(uint64_t receiver, const uint8_t *schema_ptr, uintptr_t schema_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_schema_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_schema_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_create_schema_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_schema_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_create_schema_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_update_schema(uint64_t receiver, const uint8_t *schema_ptr, uintptr_t schema_len, const uint8_t *remove_fields_ptr, uintptr_t remove_fields_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_schema_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_schema_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_update_schema_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_schema_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_update_schema_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_delete_schema(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_schema_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_schema_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_schema_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_schema_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_delete_schema_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_client_parse_client_purge_class(uint64_t receiver, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_purge_class_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_purge_class_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_client_parse_client_purge_class_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_purge_class_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_client_parse_client_purge_class_free(RustFutureHandle handle);
void boltffi_release_class_parse_core_ffi_livequery_live_query_client(uint64_t handle);
uint64_t boltffi_init_class_parse_core_ffi_livequery_live_query_client_new(const uint8_t *app_id_ptr, uintptr_t app_id_len, const uint8_t *master_key_ptr, uintptr_t master_key_len, const uint8_t *session_token_ptr, uintptr_t session_token_len, BoltFFICallbackHandle transport, BoltFFICallbackHandle listener);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_livequery_live_query_client_id(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_release(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_set_observability_sink(uint64_t receiver, BoltFFICallbackHandle sink);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_connect(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_disconnect(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_on_open(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_on_message(uint64_t receiver, const uint8_t *text_ptr, uintptr_t text_len);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_on_close(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_on_timer(uint64_t receiver, int64_t timer_id);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_on_ping(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_on_pong(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_livequery_live_query_client_subscribe(uint64_t receiver, const uint8_t *query_json_ptr, uintptr_t query_json_len, int32_t *return_out);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_livequery_live_query_client_subscribe_with_listener(uint64_t receiver, const uint8_t *query_json_ptr, uintptr_t query_json_len, BoltFFICallbackHandle listener, const uint8_t *session_token_ptr, uintptr_t session_token_len, int32_t *return_out);
FfiStatus boltffi_method_class_parse_core_ffi_livequery_live_query_client_unsubscribe(uint64_t receiver, int32_t request_id);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_livequery_live_query_client_subscribe_query(uint64_t receiver, const uint8_t *query_id_ptr, uintptr_t query_id_len, BoltFFICallbackHandle listener, const uint8_t *session_token_ptr, uintptr_t session_token_len, int32_t *return_out);
void boltffi_release_class_parse_core_ffi_object_parse_object(uint64_t handle);
FfiBuf_u8 boltffi_init_class_parse_core_ffi_object_parse_object_new(const uint8_t *client_id_ptr, uintptr_t client_id_len, const uint8_t *class_name_ptr, uintptr_t class_name_len, uint64_t *return_out);
FfiBuf_u8 boltffi_init_class_parse_core_ffi_object_parse_object_hydrate(const uint8_t *client_id_ptr, uintptr_t client_id_len, const uint8_t *class_name_ptr, uintptr_t class_name_len, const uint8_t *server_json_ptr, uintptr_t server_json_len, uint64_t *return_out);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_set_acl(uint64_t receiver, const uint8_t *acl_ptr, uintptr_t acl_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_acl(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_increment(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, double amount);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_add(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_add_unique(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_remove(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_unset(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_add_relation(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *targets_ptr, uintptr_t targets_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_remove_relation(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *targets_ptr, uintptr_t targets_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_set_reference(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *target_local_id_ptr, uintptr_t target_local_id_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_string(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_string(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_number(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, double value);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_number(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_bool(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, bool value);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_bool(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_date(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_date(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_bytes(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_bytes(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_geo(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_geo(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_file(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_file(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_pointer(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_pointer(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_polygon(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_polygon(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_set_relation(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_relation(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_dirty_keys(uint64_t receiver);
bool boltffi_method_class_parse_core_ffi_object_parse_object_is_dirty(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_get_object_ref(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_set_object_id(uint64_t receiver, const uint8_t *id_ptr, uintptr_t id_len);
bool boltffi_method_class_parse_core_ffi_object_parse_object_dirty(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_object_parse_object_revert_keys(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
bool boltffi_method_class_parse_core_ffi_object_parse_object_is_new(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_object_id(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_local_id(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_class_name(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_created_at(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_updated_at(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_object_parse_object_data_json(uint64_t receiver);
RustFutureHandle boltffi_method_class_parse_core_ffi_object_parse_object_save(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_save_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_object_parse_object_save_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_object_parse_object_save_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_save_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_save_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_object_parse_object_fetch(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_fetch_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_object_parse_object_fetch_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_object_parse_object_fetch_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_fetch_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_fetch_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_object_parse_object_destroy(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_destroy_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_object_parse_object_destroy_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_object_parse_object_destroy_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_destroy_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_object_parse_object_destroy_free(RustFutureHandle handle);
void boltffi_release_class_parse_core_ffi_query_parse_query(uint64_t handle);
FfiBuf_u8 boltffi_init_class_parse_core_ffi_query_parse_query_new(const uint8_t *client_id_ptr, uintptr_t client_id_len, const uint8_t *class_name_ptr, uintptr_t class_name_len, uint64_t *return_out);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_query_parse_query_watch_id(uint64_t receiver);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_query_parse_query_subscribe_id(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_exists(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, bool exists);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_equal_to_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_not_equal_to_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_greater_than_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_greater_than_or_equal_to_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_less_than_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_less_than_or_equal_to_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_contained_in_values(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_not_contained_in_values(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_contains_all_values(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_contained_by_values(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *values_ptr, uintptr_t values_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_near_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *point_ptr, uintptr_t point_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_greater_than_relative_time(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *spec_ptr, uintptr_t spec_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_greater_than_or_equal_to_relative_time(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *spec_ptr, uintptr_t spec_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_less_than_relative_time(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *spec_ptr, uintptr_t spec_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_less_than_or_equal_to_relative_time(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *spec_ptr, uintptr_t spec_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_within_radians_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *point_ptr, uintptr_t point_len, double max_distance, bool sorted);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_within_miles_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *point_ptr, uintptr_t point_len, double max_miles, bool sorted);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_within_kilometers_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *point_ptr, uintptr_t point_len, double max_km, bool sorted);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_within_geo_box_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *southwest_ptr, uintptr_t southwest_len, const uint8_t *northeast_ptr, uintptr_t northeast_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_within_polygon_values(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *points_ptr, uintptr_t points_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_polygon_contains_value(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *point_ptr, uintptr_t point_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_related_to_value(uint64_t receiver, const uint8_t *object_ptr, uintptr_t object_len, const uint8_t *key_ptr, uintptr_t key_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_equal_to_string(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_equal_to_number(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, double value);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_equal_to_bool(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, bool value);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_equal_to_date(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_equal_to_pointer(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_matches_query(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *where_json_ptr, uintptr_t where_json_len, const uint8_t *class_name_ptr, uintptr_t class_name_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_does_not_match_query(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *where_json_ptr, uintptr_t where_json_len, const uint8_t *class_name_ptr, uintptr_t class_name_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_matches_key_in_query(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *query_key_ptr, uintptr_t query_key_len, const uint8_t *where_json_ptr, uintptr_t where_json_len, const uint8_t *class_name_ptr, uintptr_t class_name_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_does_not_match_key_in_query(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *query_key_ptr, uintptr_t query_key_len, const uint8_t *where_json_ptr, uintptr_t where_json_len, const uint8_t *class_name_ptr, uintptr_t class_name_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_matches(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *regex_ptr, uintptr_t regex_len, const uint8_t *options_ptr, uintptr_t options_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_contains(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *substring_ptr, uintptr_t substring_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_starts_with(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *prefix_ptr, uintptr_t prefix_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_ends_with(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *suffix_ptr, uintptr_t suffix_len);
FfiBuf_u8 boltffi_method_class_parse_core_ffi_query_parse_query_where_json(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_or(uint64_t receiver, const uint8_t *clauses_ptr, uintptr_t clauses_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_and(uint64_t receiver, const uint8_t *clauses_ptr, uintptr_t clauses_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_nor(uint64_t receiver, const uint8_t *clauses_ptr, uintptr_t clauses_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_limit(uint64_t receiver, int64_t n);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_skip(uint64_t receiver, int64_t n);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_add_order(uint64_t receiver, const uint8_t *order_ptr, uintptr_t order_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_order(uint64_t receiver, const uint8_t *order_ptr, uintptr_t order_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_ascending(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_descending(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_add_ascending(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_add_descending(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_include_all(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_include(uint64_t receiver, const uint8_t *include_ptr, uintptr_t include_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_from_local_datastore(uint64_t receiver, const uint8_t *pin_name_ptr, uintptr_t pin_name_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_select(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_select_keys(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_exclude(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_watch(uint64_t receiver, const uint8_t *keys_ptr, uintptr_t keys_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_full_text(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *term_ptr, uintptr_t term_len, const uint8_t *options_ptr, uintptr_t options_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_sort_by_text_score(uint64_t receiver);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_hint(uint64_t receiver, const uint8_t *value_ptr, uintptr_t value_len);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_set_cache_policy(uint64_t receiver, ___CachePolicy policy);
FfiStatus boltffi_method_class_parse_core_ffi_query_parse_query_set_max_cache_age_ms(uint64_t receiver, int64_t ms);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_find_cached(uint64_t receiver);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_cached_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_find_cached_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_find_cached_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_cached_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_cached_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_find(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_find_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_find_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_find_with_count(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_with_count_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_find_with_count_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_find_with_count_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_with_count_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_find_with_count_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_explain(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_explain_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_explain_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_explain_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_explain_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_explain_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_next_batch(uint64_t receiver, const uint8_t *after_object_id_ptr, uintptr_t after_object_id_len, int64_t batch_size, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_next_batch_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_next_batch_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_next_batch_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_next_batch_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_next_batch_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_first(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_first_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_first_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_first_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_first_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_first_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_distinct(uint64_t receiver, const uint8_t *key_ptr, uintptr_t key_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_distinct_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_distinct_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_distinct_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_distinct_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_distinct_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_aggregate(uint64_t receiver, const uint8_t *pipeline_ptr, uintptr_t pipeline_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_aggregate_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_aggregate_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_aggregate_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_aggregate_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_aggregate_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_count(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_count_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_count_complete(RustFutureHandle handle, FfiStatus *out_status, int64_t *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_count_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_count_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_count_free(RustFutureHandle handle);
RustFutureHandle boltffi_method_class_parse_core_ffi_query_parse_query_get(uint64_t receiver, const uint8_t *object_id_ptr, uintptr_t object_id_len, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_get_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_get_complete(RustFutureHandle handle, FfiStatus *out_status, FfiBuf_u8 *return_out);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_query_parse_query_get_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_get_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_query_parse_query_get_free(RustFutureHandle handle);
void boltffi_release_class_parse_core_ffi_watch_watch_handle(uint64_t handle);
FfiBuf_u8 boltffi_init_class_parse_core_ffi_watch_watch_handle_start(const uint8_t *query_id_ptr, uintptr_t query_id_len, const uint8_t *live_query_id_ptr, uintptr_t live_query_id_len, BoltFFICallbackHandle listener, uint64_t *return_out);
RustFutureHandle boltffi_method_class_parse_core_ffi_watch_watch_handle_seed(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_watch_watch_handle_seed_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_watch_watch_handle_seed_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_watch_watch_handle_seed_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_watch_watch_handle_seed_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_watch_watch_handle_seed_free(RustFutureHandle handle);
FfiStatus boltffi_method_class_parse_core_ffi_watch_watch_handle_unsubscribe(uint64_t receiver);
RustFutureHandle boltffi_method_class_parse_core_ffi_watch_watch_handle_resync(uint64_t receiver, const uint8_t *options_ptr, uintptr_t options_len);
void boltffi_async_method_class_parse_core_ffi_watch_watch_handle_resync_poll(RustFutureHandle handle, uint64_t callback_data, void (*callback)(uint64_t, int8_t));
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_watch_watch_handle_resync_complete(RustFutureHandle handle, FfiStatus *out_status);
FfiBuf_u8 boltffi_async_method_class_parse_core_ffi_watch_watch_handle_resync_panic_message(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_watch_watch_handle_resync_cancel(RustFutureHandle handle);
void boltffi_async_method_class_parse_core_ffi_watch_watch_handle_resync_free(RustFutureHandle handle);
FfiBuf_u8 boltffi_function_parse_core_ffi_client_validate_role_name(const uint8_t *name_ptr, uintptr_t name_len);
uint64_t boltffi_function_parse_core_ffi_livequery_live_query_reconnect_delay_ms(uint32_t attempt);
FfiStatus boltffi_function_parse_core_ffi_transport_set_http_transport(BoltFFICallbackHandle transport);
FfiStatus boltffi_function_parse_core_ffi_transport_set_random_source(BoltFFICallbackHandle source);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_legacy_storage(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle storage);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_network_state_listener(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle listener);
FfiStatus boltffi_function_parse_core_ffi_transport_set_client_platform(const uint8_t *tag_ptr, uintptr_t tag_len);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_sdk_version(void);
FfiStatus boltffi_function_parse_core_ffi_transport_set_timer(BoltFFICallbackHandle timer);
FfiStatus boltffi_function_parse_core_ffi_transport_fire_timer(int64_t timer_id);
FfiStatus boltffi_function_parse_core_ffi_transport_set_clock(BoltFFICallbackHandle clock);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_session_storage(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle storage);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_installation_storage(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle storage);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_eventually_queue_listener(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle listener);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_observability_sink(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle sink);
FfiBuf_u8 boltffi_function_parse_core_ffi_transport_set_key_value_storage(const uint8_t *client_id_ptr, uintptr_t client_id_len, BoltFFICallbackHandle storage);

#ifdef __cplusplus
}
#endif