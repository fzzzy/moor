object HTTP [
  import_export_id -> "http"
]
  name: "HTTP Client"
  owner: core::ARCH_WIZARD
  readable: true

  property description (owner: core::ARCH_WIZARD, flags: "rc") = "Makes HTTP requests through the curl worker.";

  method request owner: core::ARCH_WIZARD
    "Make an HTTP request through the curl worker, with the caller's permissions.";
    "Args: {method, url [, body [, headers [, options]]]}. options may hold timeout_seconds (default 30) and the worker's max_bytes, strict_utf8 and include_url.";
    "Returns the worker's {status, headers, body}, with the final URL appended when include_url is set.";
    set_task_perms(caller_perms());
    let {method, url, ?body = "", ?headers = {}, ?options = []} = args;
    typeof(method) == TYPE_STR && typeof(url) == TYPE_STR || raise(E_TYPE, "HTTP method and URL must be strings.");
    typeof(headers) == TYPE_LIST && typeof(options) == TYPE_MAP || raise(E_TYPE, "HTTP headers must be a list and options a map.");
    let timeout = 30.0;
    if (maphaskey(options, "timeout_seconds"))
      timeout = tofloat(options["timeout_seconds"]);
      options = mapdelete(options, "timeout_seconds");
    endif
    return worker_request('curl, {method, url, body, headers, options}, ["timeout_seconds" -> timeout]);
  endmethod

  method get owner: core::ARCH_WIZARD
    "Make a GET request: {url [, headers [, options]]}.";
    set_task_perms(caller_perms());
    const {url, ?headers = {}, ?options = []} = args;
    return this:request("GET", url, "", headers, options);
  endmethod

  method test_requests_need_text_and_maps owner: core::ARCH_WIZARD
    "Malformed requests are refused before any request is made.";
    $test_utils:assert_raises(E_TYPE, this, "request", {"GET", 1}, "a URL must be a string");
    $test_utils:assert_raises(E_TYPE, this, "request", {"GET", "https://example.invalid", "", "not a list"}, "headers must be a list");
    $test_utils:assert_raises(E_TYPE, this, "request", {"GET", "https://example.invalid", "", {}, {}}, "options must be a map");
    return true;
  endmethod
endobject
