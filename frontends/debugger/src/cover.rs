use std::sync::Arc;

use anyhow::Result;
#[cfg(not(target_family = "wasm"))]
use futures::AsyncReadExt as _;
use futures::future::BoxFuture;
use gpui::http_client::{HttpClient, HttpResponse};
#[cfg(not(target_family = "wasm"))]
use http_client::{AsyncBody, Request, http};

#[derive(Clone)]
pub(crate) struct ArtQuery {
    pub game: core_model::GameId,
    pub region: Option<core_model::Region>,
    pub fallback: Option<Arc<str>>,
}

pub(crate) async fn resolve(client: Arc<dyn HttpClient>, query: &ArtQuery) -> Option<Arc<str>> {
    match fetch_sly_cover(client, query).await {
        Ok(url) => Some(url.into()),
        Err(err) => {
            tracing::debug!("artwork database unavailable ({err}), using fallback");
            query.fallback.clone()
        }
    }
}

async fn fetch_sly_cover(client: Arc<dyn HttpClient>, query: &ArtQuery) -> Result<String> {
    let url = artwork::layle::metadata_url(&query.game);
    let response = client.get(&url, true).await?;
    if !response.status.is_success() {
        return Err(anyhow::anyhow!("status {}", response.status));
    }

    let metadata = artwork::layle::parse_metadata(&response.body)?;
    let art = artwork::layle::select_art(&metadata.art, "box-front", query.region.as_slice())
        .ok_or_else(|| anyhow::anyhow!("no box front art for {}", query.game.sha1))?;
    Ok(artwork::layle::art_url(&query.game, &art))
}

#[cfg(not(target_family = "wasm"))]
pub(crate) struct ArtAuthClient(pub Arc<dyn http_client::HttpClient>);

#[cfg(not(target_family = "wasm"))]
impl HttpClient for ArtAuthClient {
    fn get(&self, url: &str, follow_redirects: bool) -> BoxFuture<'static, Result<HttpResponse>> {
        let client = self.0.clone();
        let url = url.to_owned();
        Box::pin(async move {
            let is_art = artwork::layle::is_art_url(&url);

            let mut req = Request::get(&url).body(AsyncBody::empty())?;
            if !follow_redirects || is_art {
                req.extensions_mut()
                    .insert(http_client::RedirectPolicy::NoFollow);
            }

            if is_art
                && let Some(auth) = settings::authorization()
                && let Ok(mut value) = http::HeaderValue::from_str(&auth)
            {
                value.set_sensitive(true);
                req.headers_mut().insert(http::header::AUTHORIZATION, value);
            }

            let mut response = client.send(req).await?;
            let status = response.status();
            let mut body = Vec::new();
            response.body_mut().read_to_end(&mut body).await?;

            Ok(HttpResponse { status, body })
        })
    }
}

#[cfg(target_family = "wasm")]
pub(crate) struct ArtAuthClient;

#[cfg(target_family = "wasm")]
impl HttpClient for ArtAuthClient {
    fn get(&self, url: &str, follow_redirects: bool) -> BoxFuture<'static, Result<HttpResponse>> {
        use wasm_bindgen::JsCast as _;
        use wasm_bindgen_futures::JsFuture;

        let url = url.to_owned();
        let (sender, receiver) = futures::channel::oneshot::channel();

        wasm_bindgen_futures::spawn_local(async move {
            let fetch = async {
                let js_error = |err| anyhow::anyhow!("fetch: {err:?}");
                let is_art = artwork::layle::is_art_url(&url);

                let init = web_sys::RequestInit::new();
                init.set_method("GET");
                if !follow_redirects || is_art {
                    init.set_redirect(web_sys::RequestRedirect::Error);
                }

                let request =
                    web_sys::Request::new_with_str_and_init(&url, &init).map_err(js_error)?;
                if is_art && let Some(auth) = settings::authorization() {
                    request
                        .headers()
                        .set("Authorization", &auth)
                        .map_err(js_error)?;
                }

                let window =
                    web_sys::window().ok_or_else(|| anyhow::anyhow!("no browser window"))?;
                let response: web_sys::Response =
                    JsFuture::from(window.fetch_with_request(&request))
                        .await
                        .map_err(js_error)?
                        .dyn_into()
                        .map_err(js_error)?;

                let status = http_client::http::StatusCode::from_u16(response.status())?;
                let bytes = JsFuture::from(response.array_buffer().map_err(js_error)?)
                    .await
                    .map_err(js_error)?;

                Ok(HttpResponse {
                    status,
                    body: js_sys::Uint8Array::new(&bytes).to_vec(),
                })
            }
            .await;

            let _ = sender.send(fetch);
        });

        Box::pin(async move { receiver.await? })
    }
}
