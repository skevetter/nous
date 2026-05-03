use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SchemaVersion::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SchemaVersion::Id)
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SchemaVersion::Version).text().not_null())
                    .col(
                        ColumnDef::new(SchemaVersion::AppliedAt)
                            .text()
                            .not_null()
                            .default(Expr::cust("(strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))")),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SchemaVersion::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum SchemaVersion {
    Table,
    Id,
    Version,
    AppliedAt,
}
