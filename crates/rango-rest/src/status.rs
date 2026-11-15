//! HTTP status codes.

pub const HTTP_200_OK: u16 = 200;
pub const HTTP_201_CREATED: u16 = 201;
pub const HTTP_202_ACCEPTED: u16 = 202;
pub const HTTP_204_NO_CONTENT: u16 = 204;
pub const HTTP_400_BAD_REQUEST: u16 = 400;
pub const HTTP_401_UNAUTHORIZED: u16 = 401;
pub const HTTP_403_FORBIDDEN: u16 = 403;
pub const HTTP_404_NOT_FOUND: u16 = 404;
pub const HTTP_405_METHOD_NOT_ALLOWED: u16 = 405;
pub const HTTP_409_CONFLICT: u16 = 409;
pub const HTTP_415_UNSUPPORTED_MEDIA_TYPE: u16 = 415;
pub const HTTP_429_TOO_MANY_REQUESTS: u16 = 429;
pub const HTTP_500_INTERNAL_SERVER_ERROR: u16 = 500;

pub fn is_success(code: u16) -> bool {
    (200..300).contains(&code)
}
pub fn is_client_error(code: u16) -> bool {
    (400..500).contains(&code)
}
pub fn is_server_error(code: u16) -> bool {
    (500..600).contains(&code)
}
