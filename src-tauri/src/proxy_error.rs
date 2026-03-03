const ERROR_KIND_AUTH_INVALID_GRANT: &str = "auth_invalid_grant";
const ERROR_KIND_AUTH_UNAUTHORIZED: &str = "auth_unauthorized";
const ERROR_KIND_AUTH_VERIFICATION_REQUIRED: &str = "auth_verification_required";
const ERROR_KIND_RATE_LIMITED: &str = "rate_limited";
const ERROR_KIND_INVALID_PROJECT: &str = "invalid_project";
const ERROR_KIND_INVALID_ARGUMENT: &str = "invalid_argument";
const ERROR_KIND_UPSTREAM_INTERNAL: &str = "upstream_internal";
const ERROR_KIND_NETWORK_ERROR: &str = "network_error";
const ERROR_KIND_UPSTREAM_ERROR: &str = "upstream_error";

pub(crate) fn classify_error_kind_from_message(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("invalid_grant") {
        return ERROR_KIND_AUTH_INVALID_GRANT.to_string();
    }
    if lower.contains("verify your account")
        || (lower.contains("permission_denied") && lower.contains("verify"))
    {
        return ERROR_KIND_AUTH_VERIFICATION_REQUIRED.to_string();
    }
    if lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("unauthorized")
        || lower.contains("unauthenticated")
        || lower.contains("invalid authentication credential")
    {
        return ERROR_KIND_AUTH_UNAUTHORIZED.to_string();
    }
    if lower.contains("http 429")
        || lower.contains("rate limit")
        || lower.contains("quota exceeded")
    {
        return ERROR_KIND_RATE_LIMITED.to_string();
    }
    if lower.contains("invalid project resource name")
        || (lower.contains("projects/") && lower.contains("invalid"))
        || lower.contains("cloudaicompanionproject")
    {
        return ERROR_KIND_INVALID_PROJECT.to_string();
    }
    if lower.contains("invalid argument") || lower.contains("status\": \"invalid_argument\"") {
        return ERROR_KIND_INVALID_ARGUMENT.to_string();
    }
    if lower.contains("internal error")
        || lower.contains("status\": \"internal\"")
        || lower.contains("http 500")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
    {
        return ERROR_KIND_UPSTREAM_INTERNAL.to_string();
    }
    if lower.contains("network error")
        || lower.contains("dns")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("timed out")
        || lower.contains("timeout")
    {
        return ERROR_KIND_NETWORK_ERROR.to_string();
    }
    ERROR_KIND_UPSTREAM_ERROR.to_string()
}

pub(crate) fn classify_error_kind_from_status(
    status: u16,
    path_query: &str,
    body_preview: &str,
) -> String {
    if status == 429 {
        return ERROR_KIND_RATE_LIMITED.to_string();
    }
    if status == 401 {
        return ERROR_KIND_AUTH_UNAUTHORIZED.to_string();
    }
    if status == 403 {
        let lower = body_preview.to_ascii_lowercase();
        if lower.contains("verify your account") {
            return ERROR_KIND_AUTH_VERIFICATION_REQUIRED.to_string();
        }
        return ERROR_KIND_AUTH_UNAUTHORIZED.to_string();
    }
    if status == 400 {
        let lower = body_preview.to_ascii_lowercase();
        if lower.contains("invalid project resource name")
            || (lower.contains("projects/") && lower.contains("invalid_argument"))
            || path_query.contains("streamGenerateContent")
        {
            return ERROR_KIND_INVALID_PROJECT.to_string();
        }
        return ERROR_KIND_INVALID_ARGUMENT.to_string();
    }
    if status >= 500 {
        return ERROR_KIND_UPSTREAM_INTERNAL.to_string();
    }
    classify_error_kind_from_message(body_preview)
}

pub(crate) fn should_disable_account_for_error_kind(kind: &str) -> bool {
    kind == ERROR_KIND_AUTH_INVALID_GRANT
}

