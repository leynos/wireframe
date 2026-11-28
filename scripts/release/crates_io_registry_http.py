"""Guard crates.io request construction and redirect targets.

Only HTTPS requests to the two public registry hosts pass this boundary.
urllib applies the same handler to redirect targets before opening them.

>>> is_registry_url("https://crates.io/api/v1")
True
>>> is_registry_url("https://example.invalid/api/v1")
False
"""

import typing as typ
import urllib.parse
import urllib.request

ALLOWED_HOSTS = frozenset({"crates.io", "static.crates.io"})


class HttpResponse(typ.Protocol):
    """Describe response methods consumed by the transport CLI."""

    status: int

    def read(self) -> bytes:
        """Return the complete response body."""

    def close(self) -> None:
        """Close the response after reading its body."""


class RegistryUrlError(OSError):
    """Report a request URL outside the fixed HTTPS host allowlist."""

    def __str__(self) -> str:
        """Explain the fixed registry transport boundary."""
        return "crates.io transport URL is outside the HTTPS host allowlist"


def _is_allowed_url(url: str, allowed_hosts: frozenset[str]) -> bool:
    """Check an HTTPS URL against a request handler's host policy.

    A malformed URL is outside the allowlist rather than an error: ``urlsplit``
    and the ``port`` property both raise ``ValueError`` for inputs such as an
    unbracketed IPv6 host or a non-numeric port, and a redirect ``Location``
    arrives here unvalidated. Returning ``False`` keeps every rejection on the
    same classified path as an ordinary disallowed host.

    Returns
    -------
    bool
        Whether the URL is HTTPS, on an allowed host, and carries no
        userinfo, explicit port, or fragment.

    Examples
    --------
    >>> _is_allowed_url("https://crates.io/api", frozenset({"crates.io"}))
    True
    >>> _is_allowed_url("https://crates.io:abc/api", frozenset({"crates.io"}))
    False
    """
    try:
        parsed = urllib.parse.urlsplit(url)
        port = parsed.port
    except ValueError:
        return False
    return (
        parsed.scheme == "https"
        and parsed.hostname in allowed_hosts
        and parsed.username is None
        and parsed.password is None
        and port is None
        and not parsed.fragment
    )


def is_registry_url(url: str) -> bool:
    """Allow only HTTPS URLs on crates.io's two public hosts.

    Parameters
    ----------
    url : str
        Absolute URL to check against the fixed registry hosts.

    Returns
    -------
    bool
        Whether the URL uses HTTPS and an allowed registry host.

    Examples
    --------
    >>> is_registry_url("https://crates.io/api/v1")
    True
    >>> is_registry_url("http://crates.io/api/v1")
    False
    """
    return _is_allowed_url(url, ALLOWED_HOSTS)


def build_request(
    url: str,
    *,
    headers: dict[str, str],
    data: bytes | None = None,
    method: str | None = None,
) -> urllib.request.Request:
    """Build an HTTP request only for an allowlisted registry URL.

    Parameters
    ----------
    url : str
        Absolute URL to validate before building the request.
    headers : dict[str, str]
        Headers copied onto the request.
    data : bytes or None, optional
        Optional request body.
    method : str or None, optional
        Optional HTTP method; urllib infers it when omitted.

    Returns
    -------
    urllib.request.Request
        Request targeting the validated registry URL.

    Raises
    ------
    RegistryUrlError
        If the URL is outside the HTTPS registry host allowlist.

    Examples
    --------
    >>> request = build_request(
    ...     "https://crates.io/api/v1/crates",
    ...     headers={"Accept": "application/json"},
    ...     data=b"frame",
    ...     method="POST",
    ... )
    >>> (request.full_url, request.get_method())
    ('https://crates.io/api/v1/crates', 'POST')
    """
    if not is_registry_url(url):
        raise RegistryUrlError
    # S310 cannot infer the fixed HTTPS host allowlist checked above.
    return urllib.request.Request(
        url, data=data, headers=dict(headers), method=method
    )


class _AllowlistedRequestHandler(urllib.request.BaseHandler):
    """Check every outgoing request, including urllib redirect targets."""

    def __init__(self, allowed_hosts: frozenset[str]) -> None:
        """Store the HTTPS hosts this opener may contact."""
        self._allowed_hosts = allowed_hosts

    def _check_request(self, request: urllib.request.Request) -> urllib.request.Request:
        """Reject an outgoing request unless its URL passes this handler's policy."""
        if not _is_allowed_url(request.full_url, self._allowed_hosts):
            raise RegistryUrlError
        return request

    http_request = _check_request
    https_request = _check_request
    ftp_request = _check_request
    file_request = _check_request


_HTTP_OPENER = urllib.request.build_opener(_AllowlistedRequestHandler(ALLOWED_HOSTS))


def default_opener(request: urllib.request.Request, *, timeout: float) -> HttpResponse:
    """Open a request while checking each URL against the registry allowlist.

    Parameters
    ----------
    request : urllib.request.Request
        Request for an allowlisted registry resource.
    timeout : float
        Maximum time to wait for the response.

    Returns
    -------
    HttpResponse
        Open response; the caller must close it after reading.

    Raises
    ------
    RegistryUrlError
        If the request or any redirect target is outside the host allowlist.

    Examples
    --------
    >>> request = urllib.request.Request("https://example.invalid/")
    >>> try:
    ...     default_opener(request, timeout=0.1)
    ... except RegistryUrlError as error:
    ...     print(error)
    crates.io transport URL is outside the HTTPS host allowlist
    """
    if not is_registry_url(request.full_url):
        raise RegistryUrlError
    try:
        return _HTTP_OPENER.open(request, timeout=timeout)
    except ValueError as error:
        # urllib's redirect handling resolves the Location header with
        # ``urljoin`` before this handler sees the target, so a malformed
        # Location raises here rather than in the allowlist check. Classify it
        # as a rejected destination so the transport reports one error type.
        raise RegistryUrlError from error
