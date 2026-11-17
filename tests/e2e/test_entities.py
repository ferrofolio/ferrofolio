"""Fictional entities that exist only in E2E test mode.

The end-to-end specs need entities with specific login flows and features.
Defining them here keeps the specs independent of the real connectors, which
can be removed at any time.
"""

from uuid import UUID

import domain.native_entities as native_entities
from domain.data_init import DatasourceInitContext
from domain.entity import EntityOrigin, EntityType, Feature, TransactionKind
from domain.global_position import ProductType
from domain.native_entity import (
    CredentialType,
    EntitySessionCategory,
    EntitySetupLoginType,
    InternalFeature,
    NativeFinancialEntity,
    PinChannel,
    PinDetails,
)
from infrastructure.repository.db import version_registry
from infrastructure.repository.db.client import DBCursor
from infrastructure.repository.db.upgrader import DBVersionMigration

# Simple automated login with e-mail and password
E2E_INVEST = NativeFinancialEntity(
    id=UUID("e2e00000-0000-0000-0000-000000000001"),
    name="E2E Invest",
    natural_id=None,
    type=EntityType.FINANCIAL_INSTITUTION,
    origin=EntityOrigin.NATIVE,
    features=[Feature.POSITION, Feature.TRANSACTIONS, Feature.HISTORIC],
    products=[ProductType.ACCOUNT, ProductType.REAL_ESTATE_CF],
    setup_login_type=EntitySetupLoginType.AUTOMATED,
    session_category=EntitySessionCategory.UNDEFINED,
    credentials_template={
        "user": CredentialType.EMAIL,
        "password": CredentialType.PASSWORD,
    },
    icon_url=None,
)

# Automated login with periodic contributions; the mock adds a challenge
E2E_BROKER = NativeFinancialEntity(
    id=UUID("e2e00000-0000-0000-0000-000000000002"),
    name="E2E Broker",
    natural_id=None,
    type=EntityType.FINANCIAL_INSTITUTION,
    origin=EntityOrigin.NATIVE,
    features=[Feature.POSITION, Feature.AUTO_CONTRIBUTIONS, Feature.TRANSACTIONS],
    transaction_kinds=[TransactionKind.ACCOUNT, TransactionKind.INVESTMENT],
    products=[
        ProductType.ACCOUNT,
        ProductType.CARD,
        ProductType.STOCK_ETF,
        ProductType.FUND,
        ProductType.FUND_PORTFOLIO,
        ProductType.DEPOSIT,
        ProductType.CREDIT,
    ],
    setup_login_type=EntitySetupLoginType.AUTOMATED,
    session_category=EntitySessionCategory.UNDEFINED,
    pin=PinDetails(positions=6, channel=PinChannel.SMS),
    credentials_template={
        "user": CredentialType.ID,
        "password": CredentialType.PASSWORD,
    },
    icon_url=None,
    internal_features=[InternalFeature.TWO_FACTOR_OLD_TXS],
)

# Automated login confirmed with a 6-digit code
E2E_FACTORING = NativeFinancialEntity(
    id=UUID("e2e00000-0000-0000-0000-000000000003"),
    name="E2E Factoring",
    natural_id=None,
    type=EntityType.FINANCIAL_INSTITUTION,
    origin=EntityOrigin.NATIVE,
    features=[Feature.POSITION, Feature.TRANSACTIONS, Feature.HISTORIC],
    products=[ProductType.ACCOUNT, ProductType.FACTORING],
    setup_login_type=EntitySetupLoginType.AUTOMATED,
    session_category=EntitySessionCategory.MEDIUM,
    pin=PinDetails(positions=6, channel=PinChannel.EMAIL),
    credentials_template={
        "user": CredentialType.EMAIL,
        "password": CredentialType.PASSWORD,
    },
    icon_url=None,
)

E2E_ENTITIES = [E2E_INVEST, E2E_BROKER, E2E_FACTORING]


class E2ETestEntitiesMigration(DBVersionMigration):
    @property
    def name(self):
        return "e2e:test_entities"

    async def upgrade(self, cursor: DBCursor, context: DatasourceInitContext):
        for entity in E2E_ENTITIES:
            await cursor.execute(
                "INSERT OR IGNORE INTO entities (id, name, natural_id, type, origin) "
                "VALUES (?, ?, ?, ?, ?)",
                (
                    str(entity.id),
                    entity.name,
                    entity.natural_id,
                    entity.type.value,
                    entity.origin.value,
                ),
            )


def install_e2e_entities():
    """Register the test entities; must run before any user database is set up."""
    for entity in E2E_ENTITIES:
        if entity not in native_entities.NATIVE_ENTITIES:
            native_entities.NATIVE_ENTITIES.append(entity)

    if not any(
        isinstance(v, E2ETestEntitiesMigration) for v in version_registry.versions
    ):
        version_registry.versions.append(E2ETestEntitiesMigration())
