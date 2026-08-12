#!/usr/bin/env python3
"""Runnable U5 Discord-router fixture.

This fixture uses the real durable router and a deterministic transport
instead of a Discord credential or network.  It is intentionally executable
with only the Python standard library:

    python3 tests/fixtures/gas-city/discord/test_router.py
"""

from __future__ import annotations

import importlib.util
import os
import pathlib
import sys
import tempfile
import threading
import unittest
import urllib.error
from collections.abc import Mapping
from typing import Any
from unittest.mock import patch


ROOT = pathlib.Path(__file__).resolve().parents[4]
SCRIPT = ROOT / "nix/gas-city-contributor/pack/scripts/discord-decision.py"
SPEC = importlib.util.spec_from_file_location("gascity_discord_decision", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class FakeTransport:
    def __init__(self) -> None:
        self.send_plan: list[object] = []
        self.find_plan: list[object] = []
        self.sent: list[dict[str, object]] = []
        self.notifications: list[str] = []

    def send_prompt(self, record: Mapping[str, object]) -> dict[str, object]:
        self.sent.append(dict(record))
        if self.send_plan:
            outcome = self.send_plan.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            return dict(outcome)
        return {"message_id": str(100 + len(self.sent))}

    def find_prompt(self, _record: Mapping[str, object]) -> dict[str, object] | None:
        if self.find_plan:
            outcome = self.find_plan.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            return dict(outcome) if isinstance(outcome, Mapping) else None
        return None

    def notify(self, _record: Mapping[str, object], body: str) -> dict[str, object]:
        self.notifications.append(body)
        return {"id": str(900 + len(self.notifications))}


def prompt() -> dict[str, object]:
    return {
        "protocol": MODULE.PROTOCOL,
        "run_id": "run-u5",
        "bead_id": "bead-u5",
        "decision_id": "decision-u5",
        "prompt_nonce": "nonce-u5",
        "assignee": "operator-u5",
        "guild_id": "111",
        "channel_id": "222",
        "message": "Select the next bounded action.",
        "choices": ["approve", "reject"],
    }


def event(
    *,
    choice: str = "approve",
    event_id: str = "event-1",
    guild_id: str = "111",
    channel_id: str = "222",
    operator_id: str = "333",
    reply_to: str = "101",
    run_id: str = "run-u5",
    decision_id: str = "decision-u5",
    prompt_nonce: str = "nonce-u5",
    event_type: str = "MESSAGE_CREATE",
    edited: bool = False,
) -> dict[str, object]:
    return {
        "type": event_type,
        "event_id": event_id,
        "d": {
            "guild_id": guild_id,
            "channel_id": channel_id,
            "author": {"id": operator_id},
            "id": f"300{event_id[-1] if event_id[-1].isdigit() else '1'}",
            "message_reference": {"message_id": reply_to},
            "run_id": run_id,
            "decision_id": decision_id,
            "prompt_nonce": prompt_nonce,
            "content": choice,
            "edited": edited,
        },
    }


class DiscordRouterFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.transport = FakeTransport()
        self.sleeps: list[float] = []
        self.store = MODULE.DecisionStore(self.temporary.name)
        self.router = MODULE.DecisionRouter(
            self.store,
            self.transport,
            guild_id="111",
            channel_id="222",
            operator_ids={"333"},
            sleep=self.sleeps.append,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def request(self) -> dict[str, object]:
        result = self.router.request(prompt())
        self.assertEqual(result["state"], "waiting")
        self.assertEqual(result["message_id"], "101")
        return result

    def test_valid_first_answer_cas_ack_and_duplicate_are_durable(self) -> None:
        self.request()
        pending = self.router.answer(event())
        self.assertEqual(pending["router_status"], "pending")
        self.assertEqual(self.router.wait("run-u5", "decision-u5")["choice"], "approve")
        accepted = self.router.acknowledge(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
            accepted=True,
        )
        self.assertEqual(accepted["router_status"], "accepted")
        duplicate = self.router.answer(event(event_id="event-2"))
        self.assertEqual(duplicate["router_status"], "duplicate")
        self.assertEqual(self.store.get("run-u5", "decision-u5")["event_id"], "event-1")
        self.assertEqual(
            list(pathlib.Path(self.temporary.name).rglob("*approval*")),
            [],
        )

    def test_conflicting_pending_and_answer_are_rejected(self) -> None:
        self.request()
        self.router.answer(event())
        with self.assertRaises(MODULE.ConflictError):
            self.router.answer(event(choice="reject", event_id="event-2"))
        self.router.acknowledge(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
            accepted=True,
        )
        with self.assertRaises(MODULE.ConflictError):
            self.router.answer(event(choice="reject", event_id="event-3"))

    def test_concurrent_conflicting_replies_stage_only_one_choice(self) -> None:
        self.request()
        barrier = threading.Barrier(2)
        outcomes: list[object] = []

        def answer_one(choice: str, event_id: str) -> None:
            barrier.wait()
            try:
                outcomes.append(self.router.answer(event(choice=choice, event_id=event_id)))
            except BaseException as error:
                outcomes.append(error)

        first = threading.Thread(target=answer_one, args=("approve", "event-a"))
        second = threading.Thread(target=answer_one, args=("reject", "event-b"))
        first.start()
        second.start()
        first.join()
        second.join()
        self.assertEqual(sum(isinstance(item, dict) for item in outcomes), 1)
        self.assertEqual(sum(isinstance(item, MODULE.ConflictError) for item in outcomes), 1)
        pending = self.store.get("run-u5", "decision-u5")["pending_answer"]
        self.assertIn(pending["choice"], {"approve", "reject"})

    def test_rejected_gate_precondition_leaves_no_answer_and_allows_retry(self) -> None:
        self.request()
        self.router.answer(event())
        rejected = self.router.acknowledge(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
            accepted=False,
        )
        self.assertEqual(rejected["router_status"], "rejected")
        record = self.store.get("run-u5", "decision-u5")
        self.assertEqual(record["state"], "waiting")
        self.assertIsNone(record["pending_answer"])
        self.router.answer(event(choice="reject", event_id="event-2"))

    def test_guild_channel_operator_reply_run_decision_and_choice_are_checked(self) -> None:
        self.request()
        invalid = [
            {"guild_id": "999"},
            {"channel_id": "999"},
            {"operator_id": "999"},
            {"reply_to": "999"},
            {"run_id": "other-run"},
            {"decision_id": "other-decision"},
            {"prompt_nonce": "other-nonce"},
            {"choice": "maybe"},
            {"event_type": "MESSAGE_UPDATE"},
            {"edited": True},
        ]
        for change in invalid:
            with self.subTest(change=change):
                with self.assertRaises(MODULE.DecisionError):
                    self.router.answer(event(**change))
        with self.assertRaises(MODULE.DecisionError):
            self.router.answer({"event_id": "orphan", "d": {"content": "approve"}})

    def test_delivery_retry_ceiling_and_retry_after_hints_are_durable(self) -> None:
        self.transport.send_plan = [
            MODULE.RetryableDiscordError("rate limited", retry_after=1.5),
            MODULE.RetryableDiscordError("rate limited", retry_after=2.5),
            MODULE.RetryableDiscordError("rate limited", retry_after=3.5),
        ]
        with self.assertRaises(MODULE.DecisionError):
            self.router.request(prompt())
        record = self.store.get("run-u5", "decision-u5")
        self.assertEqual(record["state"], "delivery-failed")
        self.assertEqual(record["delivery_attempts"], MODULE.MAX_DELIVERY_ATTEMPTS)
        self.assertEqual(self.sleeps, [1.5, 2.5])
        with self.assertRaises(MODULE.DecisionError):
            self.router.request(prompt())
        self.assertEqual(len(self.transport.sent), 3)

    def test_prior_delivery_reconciliation_failure_does_not_send_a_duplicate(self) -> None:
        record = self.store.ensure_prompt(prompt())
        record["delivery_attempts"] = 1
        self.store.update(record)
        self.transport.find_plan = [MODULE.DecisionError("lookup failed")]

        with self.assertRaises(MODULE.DecisionError):
            self.router.request(prompt())

        self.assertEqual(self.transport.sent, [])
        record = self.store.get("run-u5", "decision-u5")
        self.assertEqual(record["state"], "delivery-failed")
        self.assertEqual(record["delivery_error"], "permanent-reconciliation-failure")

    def test_reconciliation_retries_until_absence_before_resending(self) -> None:
        record = self.store.ensure_prompt(prompt())
        record["delivery_attempts"] = 1
        self.store.update(record)
        self.transport.find_plan = [
            MODULE.RetryableDiscordError("lookup unavailable", retry_after=1.25),
            None,
        ]
        self.transport.send_plan = [{"message_id": "779"}]

        result = self.router.request(prompt())

        self.assertEqual(result["message_id"], "779")
        self.assertEqual(len(self.transport.sent), 1)
        self.assertEqual(self.sleeps, [1.25])

    def test_reconciliation_retry_ceiling_does_not_resend(self) -> None:
        record = self.store.ensure_prompt(prompt())
        record["delivery_attempts"] = 1
        self.store.update(record)
        self.transport.find_plan = [
            MODULE.RetryableDiscordError("lookup unavailable", retry_after=1.0),
            MODULE.RetryableDiscordError("lookup unavailable", retry_after=2.0),
            MODULE.RetryableDiscordError("lookup unavailable", retry_after=3.0),
        ]

        with self.assertRaises(MODULE.DecisionError):
            self.router.request(prompt())

        self.assertEqual(self.transport.sent, [])
        self.assertEqual(self.sleeps, [1.0, 2.0])
        record = self.store.get("run-u5", "decision-u5")
        self.assertEqual(record["delivery_error"], "reconciliation-retry-ceiling")

    def test_ambiguous_send_reconciles_before_retrying(self) -> None:
        self.transport.send_plan = [
            MODULE.AmbiguousSend("connection reset", retry_after=4.0),
        ]
        self.transport.find_plan = [{"message_id": "777"}]
        result = self.router.request(prompt())
        self.assertEqual(result["message_id"], "777")
        self.assertEqual(len(self.transport.sent), 1)
        self.assertEqual(self.sleeps, [])

    def test_ambiguous_reconciliation_failure_does_not_send_a_duplicate(self) -> None:
        self.transport.send_plan = [
            MODULE.AmbiguousSend("connection reset", retry_after=4.0),
        ]
        self.transport.find_plan = [MODULE.DecisionError("lookup failed")]

        with self.assertRaises(MODULE.DecisionError):
            self.router.request(prompt())

        self.assertEqual(len(self.transport.sent), 1)
        record = self.store.get("run-u5", "decision-u5")
        self.assertEqual(record["state"], "delivery-failed")
        self.assertEqual(record["delivery_error"], "permanent-reconciliation-failure")

    def test_ambiguous_send_retries_after_exact_reconciliation_miss(self) -> None:
        self.transport.send_plan = [
            MODULE.AmbiguousSend("connection reset", retry_after=4.0),
            {"message_id": "778"},
        ]
        self.transport.find_plan = [None]
        result = self.router.request(prompt())
        self.assertEqual(result["message_id"], "778")
        self.assertEqual(len(self.transport.sent), 2)
        self.assertEqual(self.sleeps, [4.0])

    def test_wait_timeout_contract_rejects_malformed_and_out_of_range_values(self) -> None:
        for value in ("not-a-number", None, True, float("nan"), -1.0, 3600.1):
            with self.subTest(value=value):
                with self.assertRaises(MODULE.DecisionError):
                    MODULE._wait_timeout(value)
        self.assertEqual(MODULE._wait_timeout(3600), 3600.0)

    def test_wait_rpc_timeout_exceeds_wait_without_sleeping_for_the_wait(self) -> None:
        captured: dict[str, object] = {}

        def fake_rpc(
            _socket_path: str,
            request: dict[str, object],
            *,
            timeout: float,
        ) -> dict[str, object]:
            captured["request"] = request
            captured["timeout"] = timeout
            return {"protocol": MODULE.PROTOCOL, "ok": True, "result": {"router_status": "waiting"}}

        with patch.object(MODULE, "_rpc", side_effect=fake_rpc):
            self.assertEqual(
                MODULE.main(
                    [
                        "wait",
                        "--socket",
                        "unused",
                        "--run-id",
                        "run-u5",
                        "--decision-id",
                        "decision-u5",
                        "--timeout",
                        "31",
                    ]
                ),
                0,
            )

        self.assertEqual(captured["request"]["timeout"], 31.0)
        self.assertEqual(captured["timeout"], 31.0 + MODULE.WAIT_RPC_GRACE_SECONDS)

    def test_restart_returns_answered_open_prompt_without_resending(self) -> None:
        self.request()
        self.router.answer(event())
        self.router.acknowledge(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
            accepted=True,
        )
        restarted = MODULE.DecisionRouter(
            self.store,
            self.transport,
            guild_id="111",
            channel_id="222",
            operator_ids={"333"},
        )
        result = restarted.request(prompt())
        self.assertEqual(result["router_status"], "duplicate")
        self.assertEqual(len(self.transport.sent), 1)
        open_records = restarted.reconcile()
        self.assertEqual(open_records[0]["event_id"], "event-1")
        self.assertEqual(open_records[0]["answer"], "approve")

    def test_publication_notice_uses_channel_without_decision_state(self) -> None:
        result = self.router.notify_publication("Published https://github.com/acme/repo/pull/7")
        self.assertEqual(result["id"], "901")
        self.assertEqual(
            self.transport.notifications,
            ["Published https://github.com/acme/repo/pull/7"],
        )
        self.assertEqual(list(pathlib.Path(self.temporary.name, "prompts").glob("*.json")), [])

    def test_gate_close_is_durable_and_duplicate_safe(self) -> None:
        self.request()
        self.router.answer(event())
        self.router.acknowledge(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
            accepted=True,
        )
        closed = self.router.close(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
        )
        self.assertEqual(closed["router_status"], "closed")
        duplicate = self.router.close(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
        )
        self.assertEqual(duplicate["router_status"], "duplicate")
        self.assertEqual(self.store.get("run-u5", "decision-u5")["state"], "closed")

    def test_rejected_pending_answer_cannot_reopen_the_gate(self) -> None:
        self.request()
        self.router.answer(event())
        rejected = self.router.reject(
            "run-u5",
            "decision-u5",
            event_id="event-1",
            choice="approve",
        )
        self.assertEqual(rejected["router_status"], "rejected")
        with self.assertRaises(MODULE.ConflictError):
            self.router.answer(event(choice="reject", event_id="event-2"))

    def test_rest_rate_limit_hint_permanent_4xx_and_ambiguous_5xx_are_distinct(self) -> None:
        def rate_limited(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://discord.com/api/v10/channels/222/messages",
                429,
                "rate limited",
                {"Retry-After": "2.25"},
                None,
            )

        rest = MODULE.DiscordREST(
            "fixture-token",
            opener=rate_limited,
            sleep=lambda _seconds: None,
        )
        with self.assertRaises(MODULE.RetryableDiscordError) as rate_error:
            rest.request_once("POST", "/channels/222/messages", payload={"content": "x"})
        self.assertEqual(rate_error.exception.retry_after, 2.25)

        def permanent(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://discord.com/api/v10/channels/222/messages",
                400,
                "bad request",
                {},
                None,
            )

        rest.opener = permanent
        with self.assertRaises(MODULE.DecisionError):
            rest.request_once("POST", "/channels/222/messages", payload={"content": "x"})

        def ambiguous(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://discord.com/api/v10/channels/222/messages",
                502,
                "upstream",
                {"Retry-After": "3"},
                None,
            )

        rest.opener = ambiguous
        with self.assertRaises(MODULE.AmbiguousSend) as ambiguous_error:
            rest.request_once("POST", "/channels/222/messages", payload={"content": "x"})
        self.assertEqual(ambiguous_error.exception.retry_after, 3.0)

    def test_default_rest_transport_requires_the_loopback_egress_proxy(self) -> None:
        proxy = "http://127.0.0.1:3128"
        with patch.dict(
            os.environ,
            {
                "HTTP_PROXY": proxy,
                "HTTPS_PROXY": proxy,
            },
            clear=True,
        ):
            rest = MODULE.DiscordREST("fixture-token")
        opener = rest.opener.__self__
        handlers = [
            handler
            for handler in opener.handlers
            if isinstance(handler, MODULE.urllib.request.ProxyHandler)
        ]
        self.assertEqual(len(handlers), 1)
        self.assertEqual(handlers[0].proxies, {"http": proxy, "https": proxy})

        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(MODULE.DecisionError):
                MODULE.DiscordREST("fixture-token")

    def test_gateway_source_requires_tls_12(self) -> None:
        script = SCRIPT.read_text()
        self.assertIn(
            "context = ssl.create_default_context()\n"
            "        context.minimum_version = ssl.TLSVersion.TLSv1_2\n"
            "        connection = context.wrap_socket",
            script,
        )

    def test_sidecar_source_has_no_direct_network_fallback(self) -> None:
        service = (ROOT / "nixos-modules/gas-city-contributor/service.nix").read_text()
        network = (ROOT / "nixos-modules/gas-city-contributor/network.nix").read_text()
        script = SCRIPT.read_text()
        self.assertNotIn("PrivateNetwork = false", service)
        self.assertGreaterEqual(service.count("fdproxy-sidecar"), 2)
        self.assertIn("HTTPS_PROXY=http://127.0.0.1:3128", service)
        self.assertIn("StateDirectoryQuota = toString cfg.storage.discordQuotaBytes", service)
        self.assertIn(
            "discordQuotaBytes",
            (ROOT / "nixos-modules/gas-city-contributor/options.nix").read_text(),
        )
        for required in (
            "config.users.users.gascity-discord.uid",
            "config.users.users.gascity-publisher.uid",
            "discord.com",
            "gateway.discord.gg",
            "api.github.com",
            "github.com",
        ):
            self.assertIn(required, network)
        self.assertNotIn("ProxyHandler({})", script)
        self.assertNotIn("socket.create_connection((parsed.hostname", script)
        self.assertIn("CONNECT {authority}:{port}", script)


if __name__ == "__main__":
    unittest.main()
