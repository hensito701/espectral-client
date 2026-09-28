"""espectral-id app factory.

Run:  uvicorn app.main:app --host 127.0.0.1 --port 8091
"""
from __future__ import annotations

import asyncio
import logging
from contextlib import asynccontextmanager

import httpx
from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse

from . import config
from .auth import RateLimiter
from .db import Db
from .errors import ApiError
from .presence import PresenceStore
from .roles import MemberRoleCache
from .routes import router
from .signing import load_signing_key

log = logging.getLogger("espectral-id")


def _error_body(code: str, message: str) -> dict:
    return {"error": {"code": code, "message": message}}


def create_app(settings: config.Settings | None = None) -> FastAPI:
    settings = settings or config.load_settings()

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        app.state.http = httpx.AsyncClient()

        async def _sweeper():
            while True:
                await asyncio.sleep(300)
                app.state.db.purge_expired()
                app.state.presence.purge()

        task = asyncio.create_task(_sweeper())
        yield
        task.cancel()
        await app.state.http.aclose()

    app = FastAPI(title="espectral-id", docs_url=None, redoc_url=None,
                  openapi_url=None, lifespan=lifespan)
    app.state.settings = settings
    app.state.db = Db(settings.db_path)
    app.state.presence = PresenceStore(config.PRESENCE_TTL_SECONDS)
    app.state.member_cache = MemberRoleCache()
    app.state.limiter = RateLimiter()

    if settings.signing_key_b64:
        app.state.signing_key = load_signing_key(settings.signing_key_b64)
    else:
        log.warning("ESPECTRAL_ID_SIGNING_KEY unset: grants cannot be issued")
        app.state.signing_key = None

    app.include_router(router)

    @app.exception_handler(ApiError)
    async def _api_error(_request: Request, exc: ApiError):
        return JSONResponse(_error_body(exc.code, exc.message),
                            status_code=exc.status)

    @app.exception_handler(RequestValidationError)
    async def _validation_error(_request: Request, exc: RequestValidationError):
        return JSONResponse(_error_body("bad_request", "Malformed request"),
                            status_code=400)

    @app.exception_handler(404)
    async def _not_found(_request: Request, exc):
        return JSONResponse(_error_body("not_found", "Not found"),
                            status_code=404)

    @app.exception_handler(500)
    async def _internal(_request: Request, exc):
        log.exception("unhandled error")
        return JSONResponse(_error_body("internal", "Internal error"),
                            status_code=500)

    return app


app = create_app()
